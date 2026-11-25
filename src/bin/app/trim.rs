use std::path::{Path, PathBuf};

use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::Retained;
use objc2_av_foundation::{
    AVAssetExportPresetPassthrough, AVAssetExportSession, AVAssetExportSessionStatus, AVAssetTrack,
    AVFileType, AVFileTypeAppleM4V, AVFileTypeMPEG4, AVFileTypeQuickTimeMovie, AVPlayerItem, AVURLAsset,
};
use objc2_av_kit::AVPlayerViewTrimResult;
use objc2_foundation::{NSArray, NSString};
use view_core::{validate_trim_selection, trim_selection_is_full, FileIdentity, TrimTime};

use super::{app_state, focused_item, path_from_url, player, reconcile_after_mutation, show_error_alert, update_carousel_preview, url_from_path, window_refs, MediaKind, ViewMode};

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub(crate) struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

impl CMTime {
    fn invalid() -> Self {
        Self { value: 0, timescale: 0, flags: 0, epoch: 0 }
    }

    fn valid(value: i64, timescale: i32) -> Self {
        Self { value, timescale, flags: 1, epoch: 0 }
    }

    fn trim_time(self) -> Option<TrimTime> {
        let implied = (1 << 2) | (1 << 3) | (1 << 4);
        if self.flags & 1 == 0 || self.flags & implied != 0 || self.timescale <= 0 {
            return None;
        }
        Some(TrimTime { value: self.value, timescale: self.timescale, epoch: self.epoch })
    }
}

unsafe impl Encode for CMTime {
    const ENCODING: Encoding = Encoding::Struct(
        "?",
        &[<i64 as Encode>::ENCODING, <i32 as Encode>::ENCODING, <u32 as Encode>::ENCODING, <i64 as Encode>::ENCODING],
    );
}

unsafe impl RefEncode for CMTime {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CMTimeRange {
    start: CMTime,
    duration: CMTime,
}

unsafe impl Encode for CMTimeRange {
    const ENCODING: Encoding = Encoding::Struct("?", &[CMTime::ENCODING, CMTime::ENCODING]);
}

unsafe impl RefEncode for CMTimeRange {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Trimming,
    AwaitingSave,
    Exporting,
    Validating,
}

#[derive(Clone)]
enum SaveTarget {
    ReplaceOriginal,
    Copy(PathBuf),
}

struct TrimOperation {
    token: u64,
    source: PathBuf,
    identity: FileIdentity,
    item: Retained<AVPlayerItem>,
    item_pointer: usize,
    playhead: CMTime,
    phase: Phase,
    selection: Option<view_core::TrimSelection>,
    export: Option<Retained<AVAssetExportSession>>,
    temp_dir: Option<PathBuf>,
    target: Option<SaveTarget>,
}

thread_local! {
    static OPERATION: std::cell::UnsafeCell<Option<TrimOperation>> = const { std::cell::UnsafeCell::new(None) };
}
static NEXT_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn operation() -> &'static mut Option<TrimOperation> {
    OPERATION.with(|cell| unsafe { &mut *cell.get() })
}

pub(crate) fn active() -> bool {
    operation().is_some()
}

fn finish() {
    let op = operation().take();
    if let Some(op) = op {
        let playhead = op.playhead;
        unsafe {
            let _: () = objc2::msg_send![&*op.item, setForwardPlaybackEndTime: CMTime::invalid()];
            let _: () = objc2::msg_send![&*op.item, setReversePlaybackEndTime: CMTime::invalid()];
        }
        if let Some(temp) = op.temp_dir {
            let _ = std::fs::remove_dir_all(temp);
        }
        restore_playhead(playhead);
    }
    player::set_trim_focus(false);
    restore_grid_focus();
}

fn restore_grid_focus() {
    if let Some(refs) = window_refs().as_ref() {
        unsafe {
            let window = &*refs.window;
            let grid = &*refs.grid;
            let _ = window.makeFirstResponder(Some(grid));
        }
    }
}

fn fail(message: &str) {
    finish();
    show_error_alert("Unable to trim video", message);
}

fn reverse_playback_end_time(item: &AVPlayerItem) -> CMTime {
    unsafe { objc2::msg_send![item, reversePlaybackEndTime] }
}

fn forward_playback_end_time(item: &AVPlayerItem) -> CMTime {
    unsafe { objc2::msg_send![item, forwardPlaybackEndTime] }
}

fn item_duration(item: &AVPlayerItem) -> CMTime {
    unsafe { objc2::msg_send![item, duration] }
}

fn player_current_time() -> CMTime {
    player::with_store(|store| unsafe { objc2::msg_send![&*store.player, currentTime] }).unwrap_or_else(CMTime::invalid)
}

fn restore_playhead(playhead: CMTime) {
    if playhead.trim_time().is_none() {
        return;
    }
    player::with_store(|store| unsafe {
        let _: () = objc2::msg_send![&*store.player, seekToTime: playhead];
    });
}

fn container_type(path: &Path) -> Option<&'static AVFileType> {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("mov") => unsafe { AVFileTypeQuickTimeMovie },
        Some("mp4") => unsafe { AVFileTypeMPEG4 },
        Some("m4v") => unsafe { AVFileTypeAppleM4V },
        _ => None,
    }
}

fn supported_container(path: &Path) -> bool {
    container_type(path).is_some()
}

pub(crate) fn begin_selected() {
    if active() {
        return;
    }
    let (item_path, ready, can_trim) = {
        let state = unsafe { &*app_state() };
        let Some(media) = focused_item(state) else { return };
        if media.kind != MediaKind::Movie || state.tab().catalog.selected_all().len() != 1 || state.pending_trash.contains(&media.path) || !supported_container(&media.path) {
            return;
        }
        let ready = player::with_store(|store| {
            store.status == player::PlayerStatus::Ready
                && store.item_path.as_deref() == Some(media.path.as_path())
                && store.item.is_some()
        }).unwrap_or(false);
        let can_trim = player::with_store(|store| unsafe { store.view.canBeginTrimming() }).unwrap_or(false);
        (media.path, ready, can_trim)
    };
    if !ready || !can_trim {
        return;
    }
    if unsafe { (*app_state()).tab().view_mode } == ViewMode::Grid {
        unsafe { (*app_state()).tab_mut() }.view_mode = ViewMode::Carousel;
        super::apply_view_mode_layout();
    }
    let Some((item, view)) = player::with_store(|store| (store.item.clone(), store.view.clone())) else { return };
    let Some(item) = item else { return };
    let Ok(identity) = FileIdentity::capture(&item_path) else {
        show_error_alert("Unable to trim video", "The source file could not be inspected.");
        return;
    };
    player::pause();
    let playhead = player_current_time();
    let token = NEXT_TOKEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    *operation() = Some(TrimOperation {
        token,
        source: item_path.clone(),
        identity,
        item_pointer: Retained::as_ptr(&item) as usize,
        item,
        playhead,
        phase: Phase::Trimming,
        selection: None,
        export: None,
        temp_dir: None,
        target: None,
    });
    player::set_trim_focus(true);
    if let Some(refs) = window_refs().as_ref() {
        unsafe { let _ = (&*refs.window).makeFirstResponder(Some(&*view)); }
    }
    let handler = block2::RcBlock::new(move |result: AVPlayerViewTrimResult| {
        trim_completed(token, result);
    });
    unsafe { view.beginTrimmingWithCompletionHandler(Some(&handler)) };
}

fn trim_completed(token: u64, result: AVPlayerViewTrimResult) {
    if objc2::MainThreadMarker::new().is_none() {
        dispatch2::DispatchQueue::main().exec_async(move || trim_completed(token, result));
        return;
    }
    let valid = operation().as_ref().is_some_and(|op| op.token == token && op.phase == Phase::Trimming);
    if !valid {
        return;
    }
    if result == AVPlayerViewTrimResult::CancelButton {
        finish();
        return;
    }
    let extracted = operation().as_ref().and_then(|op| {
        if Retained::as_ptr(&op.item) as usize != op.item_pointer {
            return None;
        }
        let duration = item_duration(&op.item).trim_time()?;
        let start = reverse_playback_end_time(&op.item).trim_time().unwrap_or(TrimTime::new(0, duration.timescale));
        let end = forward_playback_end_time(&op.item).trim_time().unwrap_or(duration);
        validate_trim_selection(start, end, duration).ok()
    });
    let Some(selection) = extracted else {
        fail("The selected trimming boundaries are not valid for this movie.");
        return;
    };
    if trim_selection_is_full(selection) {
        finish();
        return;
    }
    if let Some(op) = operation().as_mut() {
        op.selection = Some(selection);
        op.phase = Phase::AwaitingSave;
    }
    dispatch2::DispatchQueue::main().exec_async(move || offer_save(token));
}

fn offer_save(token: u64) {
    let ready = operation().as_ref().is_some_and(|op| op.token == token && op.phase == Phase::AwaitingSave);
    if !ready {
        return;
    }
    let source = operation().as_ref().map(|op| op.source.clone()).unwrap_or_default();
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let alert = objc2_app_kit::NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("Save trimmed video"));
    alert.setInformativeText(&NSString::from_str("Save a separate copy, or replace the original. Replacing the original cannot be undone in View."));
    alert.addButtonWithTitle(&NSString::from_str("Save Trimmed Copy…"));
    alert.addButtonWithTitle(&NSString::from_str("Replace Original…"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    if super::internal_test_mode() {
        super::schedule_test_alert_click("Replace Original…");
    }
    let response = alert.runModal();
    if response == objc2_app_kit::NSAlertFirstButtonReturn {
        let Some(destination) = choose_copy_destination(&source) else {
            finish();
            return;
        };
        if destination.exists() {
            fail("The chosen destination already exists. View will not overwrite it.");
            return;
        }
        start_export(token, SaveTarget::Copy(destination));
    } else if response == objc2_app_kit::NSAlertSecondButtonReturn {
        let confirm = objc2_app_kit::NSAlert::new(mtm);
        confirm.setMessageText(&NSString::from_str("Replace the original video?"));
        confirm.setInformativeText(&NSString::from_str("The untrimmed original will be replaced after the trimmed export validates. This cannot be undone in View."));
        confirm.addButtonWithTitle(&NSString::from_str("Replace Original"));
        confirm.addButtonWithTitle(&NSString::from_str("Cancel"));
        if super::internal_test_mode() {
            super::schedule_test_alert_click("Replace Original");
        }
        if confirm.runModal() == objc2_app_kit::NSAlertFirstButtonReturn {
            start_export(token, SaveTarget::ReplaceOriginal);
        } else {
            finish();
        }
    } else {
        finish();
    }
}

fn choose_copy_destination(source: &Path) -> Option<PathBuf> {
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let panel = objc2_app_kit::NSSavePanel::new(mtm);
    let file_type = container_type(source)?;
    if let Some(content_type) = objc2_uniform_type_identifiers::UTType::typeWithIdentifier(file_type) {
        panel.setAllowedContentTypes(&NSArray::from_slice(&[&*content_type]));
    }
    if let Some(parent) = source.parent() {
        panel.setDirectoryURL(Some(&url_from_path(parent)));
    }
    let stem = source.file_stem()?.to_string_lossy();
    let extension = source.extension()?.to_string_lossy();
    panel.setNameFieldStringValue(&NSString::from_str(&format!("{stem} Trimmed.{extension}")));
    panel.setCanCreateDirectories(true);
    if panel.runModal() != objc2_app_kit::NSModalResponseOK {
        return None;
    }
    panel.URL().map(|url| path_from_url(&url))
}

fn unique_temp_dir(parent: &Path) -> std::io::Result<PathBuf> {
    for counter in 0..1000u32 {
        let candidate = parent.join(format!(".view-trim-{}-{counter}", std::process::id()));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "could not allocate a temporary export directory"))
}

fn start_export(token: u64, target: SaveTarget) {
    let (source, selection, file_type) = {
        let Some(op) = operation().as_ref() else { return };
        if op.token != token || op.phase != Phase::AwaitingSave {
            return;
        }
        let Some(selection) = op.selection else { return };
        let Some(file_type) = container_type(&op.source) else { return };
        (op.source.clone(), selection, file_type)
    };
    if let Err(error) = (|| -> std::io::Result<()> {
        if FileIdentity::capture(&source)? != operation().as_ref().unwrap().identity {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "The source file changed after trimming started."));
        }
        Ok(())
    })() {
        fail(&error.to_string());
        return;
    }
    if let SaveTarget::Copy(path) = &target {
        let source_extension = source.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        let destination_extension = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        if source_extension != destination_extension {
            fail("The trimmed copy must keep the source filename extension because passthrough export preserves the original container.");
            return;
        }
    }
    let parent = match &target {
        SaveTarget::ReplaceOriginal => source.parent().map(Path::to_path_buf),
        SaveTarget::Copy(path) => path.parent().map(Path::to_path_buf),
    };
    let Some(parent) = parent else {
        fail("The destination has no parent directory.");
        return;
    };
    let temp_dir = match unique_temp_dir(&parent) {
        Ok(path) => path,
        Err(error) => {
            fail(&format!("Could not create a temporary export directory: {error}"));
            return;
        }
    };
    if let Some(op) = operation().as_mut() {
        op.temp_dir = Some(temp_dir.clone());
    }
    let extension = source.extension().and_then(|e| e.to_str()).unwrap_or("mov");
    let output = temp_dir.join(format!("output.{extension}"));
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url_from_path(&source), None) };
    let Some(session) = (unsafe { AVAssetExportSession::exportSessionWithAsset_presetName(&asset, AVAssetExportPresetPassthrough) }) else {
        fail("This movie cannot be exported without re-encoding.");
        return;
    };
    let type_supported = unsafe { session.supportedFileTypes() }.iter().any(|supported| &*supported == file_type);
    if !type_supported {
        fail("Passthrough export does not support this movie's container without re-encoding.");
        return;
    }
    unsafe {
        session.setOutputURL(Some(&url_from_path(&output)));
        session.setOutputFileType(Some(file_type));
        let start = CMTime::valid(selection.start.value, selection.start.timescale);
        let Some(duration_value) = rational_difference_value(selection.end, selection.start) else {
            fail("The selected trim range cannot be represented exactly by CoreMedia.");
            return;
        };
        let duration = CMTime::valid(duration_value.0, duration_value.1);
        let range = CMTimeRange { start, duration };
        let _: () = objc2::msg_send![&*session, setTimeRange: range];
    }
    if let Some(op) = operation().as_mut() {
        op.phase = Phase::Exporting;
        op.temp_dir = Some(temp_dir);
        op.target = Some(target);
        op.export = Some(session.clone());
    }
    let handler = block2::RcBlock::new(move || {
        dispatch2::DispatchQueue::main().exec_async(move || export_completed(token));
    });
    unsafe { session.exportAsynchronouslyWithCompletionHandler(&handler) };
}

fn rational_difference_value(end: TrimTime, start: TrimTime) -> Option<(i64, i32)> {
    if end.timescale == start.timescale {
        return Some((end.value - start.value, end.timescale));
    }
    let scale = i128::from(end.timescale) * i128::from(start.timescale);
    let end_scaled = i128::from(end.value) * i128::from(start.timescale);
    let start_scaled = i128::from(start.value) * i128::from(end.timescale);
    let mut numerator = end_scaled - start_scaled;
    let mut denominator = scale;
    let mut a = numerator.unsigned_abs();
    let mut b = denominator as u128;
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    let gcd = a.max(1) as i128;
    numerator /= gcd;
    denominator /= gcd;
    while denominator > i128::from(i32::MAX) && denominator % 2 == 0 && numerator % 2 == 0 {
        numerator /= 2;
        denominator /= 2;
    }
    Some((i64::try_from(numerator).ok()?, i32::try_from(denominator).ok()?))
}

fn export_completed(token: u64) {
    let (status, error) = {
        let Some(op) = operation().as_ref() else { return };
        if op.token != token || op.phase != Phase::Exporting {
            return;
        }
        let Some(session) = op.export.as_ref() else { return };
        let error = unsafe { session.error() }.map(|error| error.localizedDescription().to_string());
        (unsafe { session.status() }, error)
    };
    match status {
        AVAssetExportSessionStatus::Completed => {
            if let Some(op) = operation().as_mut() {
                op.phase = Phase::Validating;
            }
            validate_export(token);
        }
        AVAssetExportSessionStatus::Cancelled => finish(),
        _ => fail(&error.unwrap_or_else(|| "The export failed.".to_string())),
    }
}

fn validate_export(token: u64) {
    let Some((source, selection, target, temp_output, identity)) = operation().as_ref().and_then(|op| {
        if op.token != token || op.phase != Phase::Validating {
            return None;
        }
        let temp_dir = op.temp_dir.clone()?;
        let extension = op.source.extension().and_then(|e| e.to_str()).unwrap_or("mov");
        Some((op.source.clone(), op.selection?, op.target.as_ref()?.clone(), temp_dir.join(format!("output.{extension}")), op.identity))
    }) else {
        return;
    };
    std::thread::spawn(move || {
        let result = validate_exported_asset(&source, &temp_output, selection, &target, identity);
        dispatch2::DispatchQueue::main().exec_async(move || validation_completed(token, result));
    });
}

fn asset_signature(path: &Path) -> Result<(TrimTime, Vec<(String, i64, i64, usize)>), String> {
    let _pool = super::AutoreleasePool::push();
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url_from_path(path), None) };
    #[allow(deprecated)]
    unsafe {
        let duration: CMTime = objc2::msg_send![&*asset, duration];
        let Some(duration) = duration.trim_time() else {
            return Err("The movie duration is invalid.".to_string());
        };
        let tracks: Retained<NSArray<AVAssetTrack>> = objc2::msg_send![&*asset, tracks];
        let mut signature: Vec<(String, i64, i64, usize)> = tracks.iter().map(|track| {
            let size = track.naturalSize();
            (track.mediaType().to_string(), size.width as i64, size.height as i64, track.formatDescriptions().len())
        }).collect();
        signature.sort();
        Ok((duration, signature))
    }
}

fn validate_exported_asset(source: &Path, output: &Path, selection: view_core::TrimSelection, target: &SaveTarget, identity: FileIdentity) -> Result<(), String> {
    if matches!(target, SaveTarget::ReplaceOriginal) && identity.links != 1 {
        return Err("Replacing a hard-linked source is not supported.".to_string());
    }
    if !identity.matches(source).map_err(|e| e.to_string())? {
        return Err("The source file changed while the trimmed copy was exporting.".to_string());
    }
    let metadata = std::fs::metadata(output).map_err(|e| e.to_string())?;
    if metadata.len() == 0 {
        return Err("The export produced an empty file.".to_string());
    }
    let (_, source_tracks) = asset_signature(source)?;
    let (output_duration, output_tracks) = asset_signature(output)?;
    if source_tracks != output_tracks {
        return Err("The export did not preserve the source media tracks.".to_string());
    }
    let expected_numerator = i128::from(selection.end.value) * i128::from(selection.start.timescale)
        - i128::from(selection.start.value) * i128::from(selection.end.timescale);
    let expected_denominator = i128::from(selection.end.timescale) * i128::from(selection.start.timescale);
    let actual_numerator = i128::from(output_duration.value);
    let actual_denominator = i128::from(output_duration.timescale);
    let difference = (actual_numerator * expected_denominator - expected_numerator * actual_denominator).abs();
    let tolerance = actual_denominator * expected_denominator;
    if difference > tolerance {
        return Err("The exported duration differs from the selected trim range by more than one second.".to_string());
    }
    Ok(())
}

fn validation_completed(token: u64, result: Result<(), String>) {
    let valid = operation().as_ref().is_some_and(|op| op.token == token && op.phase == Phase::Validating);
    if !valid {
        return;
    }
    match result {
        Ok(()) => commit_export(token),
        Err(message) => fail(&message),
    }
}

fn move_exclusive(from: &Path, to: &Path, flags: libc::c_uint) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let from = std::ffi::CString::new(from.as_os_str().as_bytes()).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes()).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), flags) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn copy_source_metadata(source: &Path, output: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let source_c = std::ffi::CString::new(source.as_os_str().as_bytes()).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let output_c = std::ffi::CString::new(output.as_os_str().as_bytes()).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let result = unsafe {
        libc::copyfile(
            source_c.as_ptr(),
            output_c.as_ptr(),
            std::ptr::null_mut(),
            libc::COPYFILE_METADATA | libc::COPYFILE_NOFOLLOW,
        )
    };
    if result == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

fn commit_export(token: u64) {
    let Some((source, target, temp_dir)) = operation().as_ref().and_then(|op| {
        if op.token != token || op.phase != Phase::Validating {
            return None;
        }
        Some((op.source.clone(), op.target.as_ref()?.clone(), op.temp_dir.clone()?))
    }) else {
        return;
    };
    let extension = source.extension().and_then(|e| e.to_str()).unwrap_or("mov");
    let output = temp_dir.join(format!("output.{extension}"));
    let result = match &target {
        SaveTarget::Copy(destination) => move_exclusive(&output, destination, libc::RENAME_EXCL),
        SaveTarget::ReplaceOriginal => {
            let identity = operation().as_ref().unwrap().identity;
            copy_source_metadata(&source, &output)
                .and_then(|()| {
                    if identity.matches(&source)? {
                        move_exclusive(&output, &source, libc::RENAME_SWAP)
                    } else {
                        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "The source file changed before replacement."))
                    }
                })
        }
    };
    if let Err(error) = result {
        fail(&format!("Could not commit the trimmed file: {error}"));
        return;
    }
    if matches!(target, SaveTarget::ReplaceOriginal) {
        let _ = std::fs::remove_file(&output);
    }
    let changed = match &target {
        SaveTarget::ReplaceOriginal => vec![source.clone()],
        SaveTarget::Copy(destination) => vec![destination.clone()],
    };
    let select = changed.clone();
    finish();
    reconcile_after_mutation(&changed, &select);
    if matches!(target, SaveTarget::ReplaceOriginal) {
        player::reload_current();
    }
    update_carousel_preview();
}

pub(crate) fn conflicts() -> bool {
    active()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/trim-source.mp4")
    }

    #[test]
    fn cmtime_layout_reads_fixture_duration() {
        let _pool = super::super::AutoreleasePool::push();
        let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url_from_path(&fixture()), None) };
        #[allow(deprecated)]
        let duration: CMTime = unsafe { objc2::msg_send![&*asset, duration] };
        let duration = duration.trim_time().unwrap();
        assert_eq!(duration.timescale, 600);
        assert_eq!(duration.value, 3600);
    }

    #[test]
    fn passthrough_export_writes_readable_trimmed_mp4() {
        let _pool = super::super::AutoreleasePool::push();
        let source = fixture();
        let root = std::env::temp_dir().join(format!("view-trim-export-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let output = root.join("output.mp4");
        let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url_from_path(&source), None) };
        let session = unsafe { AVAssetExportSession::exportSessionWithAsset_presetName(&asset, AVAssetExportPresetPassthrough) }.unwrap();
        let file_type = container_type(&source).unwrap();
        assert!(unsafe { session.supportedFileTypes() }.iter().any(|supported| &*supported == file_type));
        unsafe {
            session.setOutputURL(Some(&url_from_path(&output)));
            session.setOutputFileType(Some(file_type));
            let range = CMTimeRange { start: CMTime::valid(1 * 600, 600), duration: CMTime::valid(3 * 600, 600) };
            let _: () = objc2::msg_send![&*session, setTimeRange: range];
        }
        static DONE: AtomicBool = AtomicBool::new(false);
        DONE.store(false, Ordering::SeqCst);
        let handler = block2::RcBlock::new(|| DONE.store(true, Ordering::SeqCst));
        unsafe { session.exportAsynchronouslyWithCompletionHandler(&handler) };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            let status = unsafe { session.status() };
            if status != AVAssetExportSessionStatus::Waiting && status != AVAssetExportSessionStatus::Exporting {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(unsafe { session.status() }, AVAssetExportSessionStatus::Completed);
        assert!(DONE.load(Ordering::SeqCst));
        let result = validate_exported_asset(
            &source,
            &output,
            validate_trim_selection(TrimTime::new(600, 600), TrimTime::new(2400, 600), TrimTime::new(3600, 600)).unwrap(),
            &SaveTarget::Copy(output.clone()),
            FileIdentity::capture(&source).unwrap(),
        );
        assert!(result.is_ok(), "{result:?}");
        let size = std::fs::metadata(&output).unwrap().len();
        assert!(size > 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
