use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, ClassType, MainThreadOnly};
use objc2_av_foundation::{AVPlayer, AVPlayerItem, AVPlayerItemStatus};
use objc2_foundation::{
    NSDictionary, NSKeyValueObservingOptions, NSObjectNSKeyValueObserverRegistration, NSObjectProtocol,
    NSString,
};

use super::{quicklook_selected, url_from_path, PreviewPlayerView};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlayerStatus {
    Loading,
    Ready,
    Failed,
}

pub(crate) struct PlayerStore {
    pub(crate) view: Retained<PreviewPlayerView>,
    pub(crate) player: Retained<AVPlayer>,
    pub(crate) observer: Retained<PlayerItemObserver>,
    pub(crate) poster: *mut objc2_app_kit::NSImageView,
    pub(crate) item: Option<Retained<AVPlayerItem>>,
    pub(crate) item_path: Option<PathBuf>,
    pub(crate) status: PlayerStatus,
    pub(crate) autoplay: bool,
}

struct MainThreadStore(std::cell::UnsafeCell<Option<PlayerStore>>);

unsafe impl Send for MainThreadStore {}
unsafe impl Sync for MainThreadStore {}

static PLAYER_STORE: std::sync::OnceLock<MainThreadStore> = std::sync::OnceLock::new();

fn player_store() -> &'static mut Option<PlayerStore> {
    let cell = PLAYER_STORE.get_or_init(|| MainThreadStore(std::cell::UnsafeCell::new(None)));
    // SAFETY: player state is created and used only on the main thread.
    unsafe { &mut *cell.0.get() }
}

pub(crate) fn install(store: PlayerStore) {
    unsafe {
        store.view.addObserver_forKeyPath_options_context(
            store.observer.as_super(),
            &NSString::from_str("readyForDisplay"),
            NSKeyValueObservingOptions::New,
            core::ptr::null_mut(),
        );
    }
    *player_store() = Some(store);
}

pub(crate) fn with_store<R>(f: impl FnOnce(&PlayerStore) -> R) -> Option<R> {
    player_store().as_ref().map(f)
}



define_class!(
    #[unsafe(super(objc2::runtime::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    pub(crate) struct PlayerItemObserver;

    impl PlayerItemObserver {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            _key_path: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&NSDictionary>,
            _context: *mut core::ffi::c_void,
        ) {
            let object = object.map(|observed| observed as *const AnyObject);
            player_item_status_changed(object);
        }
    }
);

unsafe impl NSObjectProtocol for PlayerItemObserver {}

pub(crate) fn new_observer() -> Retained<PlayerItemObserver> {
    unsafe { objc2::msg_send![PlayerItemObserver::class(), new] }
}

fn update_player_visibility(store: &mut PlayerStore) {
    let visible = store.item_path.is_some() && store.status == PlayerStatus::Ready && unsafe { store.view.isReadyForDisplay() };
    store.view.setHidden(!visible);
    unsafe { (*store.poster).setHidden(visible) };
    if *super::PLAYER_TEST.get_or_init(|| false) && visible {
        assert!(unsafe { store.view.isReadyForDisplay() }, "player must not replace the poster before its first frame is ready");
    }
}

fn player_item_status_changed(object: Option<*const AnyObject>) {
    if objc2::MainThreadMarker::new().is_none() {
        let object = object.map(|observed| observed as usize);
        dispatch2::DispatchQueue::main().exec_async(move || {
            player_item_status_changed(object.map(|observed| observed as *const AnyObject));
        });
        return;
    }
    let Some(store) = player_store().as_mut() else { return };
    if object == Some(Retained::as_ptr(&store.view) as *const AnyObject) {
        update_player_visibility(store);
        return;
    }
    let Some(item) = store.item.as_ref() else { return };
    if object != Some(Retained::as_ptr(item) as *const AnyObject) {
        return;
    }
    store.status = match unsafe { item.status() } {
        AVPlayerItemStatus::Unknown => PlayerStatus::Loading,
        AVPlayerItemStatus::ReadyToPlay => PlayerStatus::Ready,
        AVPlayerItemStatus::Failed => PlayerStatus::Failed,
        _ => PlayerStatus::Loading,
    };
    update_player_visibility(store);
    if store.status == PlayerStatus::Ready && store.autoplay {
        store.autoplay = false;
        unsafe { store.player.play() };
    }
}

pub(crate) fn release() {
    let (player, observer, old_item) = {
        let Some(store) = player_store().as_mut() else { return };
        store.item_path = None;
        store.status = PlayerStatus::Loading;
        store.autoplay = false;
        update_player_visibility(store);
        (store.player.clone(), store.observer.clone(), store.item.take())
    };
    if let Some(old_item) = old_item {
        unsafe {
            let key = NSString::from_str("status");
            old_item
                .as_super()
                .removeObserver_forKeyPath_context(observer.as_super(), &key, core::ptr::null_mut());
        }
    }
    unsafe { player.replaceCurrentItemWithPlayerItem(None) };
    if let Some(store) = player_store().as_mut() {
        update_player_visibility(store);
    }
}

pub(crate) fn load(path: &std::path::Path) {
    let Some(mtm) = objc2::MainThreadMarker::new() else { return };
    let (player, observer, old_item) = {
        let Some(store) = player_store().as_mut() else { return };
        if store.item_path.as_deref() == Some(path) {
            return;
        }
        store.item_path = None;
        store.status = PlayerStatus::Loading;
        store.autoplay = false;
        update_player_visibility(store);
        (store.player.clone(), store.observer.clone(), store.item.take())
    };
    if let Some(old_item) = old_item {
        unsafe {
            let key = NSString::from_str("status");
            old_item
                .as_super()
                .removeObserver_forKeyPath_context(observer.as_super(), &key, core::ptr::null_mut());
        }
    }
    let url = url_from_path(path);
    let item = unsafe { AVPlayerItem::playerItemWithURL(&url, mtm) };
    unsafe {
        let key = NSString::from_str("status");
        item.as_super().addObserver_forKeyPath_options_context(
            observer.as_super(),
            &key,
            NSKeyValueObservingOptions::New,
            core::ptr::null_mut(),
        );
    }
    unsafe { player.replaceCurrentItemWithPlayerItem(Some(&item)) };
    let Some(store) = player_store().as_mut() else { return };
    store.item = Some(item);
    store.item_path = Some(path.to_path_buf());
    store.autoplay = false;
    store.status = match unsafe { store.item.as_ref().unwrap().status() } {
        AVPlayerItemStatus::Unknown => PlayerStatus::Loading,
        AVPlayerItemStatus::ReadyToPlay => PlayerStatus::Ready,
        AVPlayerItemStatus::Failed => PlayerStatus::Failed,
        _ => PlayerStatus::Loading,
    };
    update_player_visibility(store);
}

pub(crate) fn start() {
    let Some(store) = player_store().as_mut() else { return };
    if store.item_path.is_none() {
        return;
    }
    if store.status == PlayerStatus::Ready {
        store.autoplay = false;
        unsafe { store.player.play() };
    } else {
        store.autoplay = true;
    }
}

pub(crate) fn pause() {
    let Some(store) = player_store().as_mut() else { return };
    store.autoplay = false;
    if unsafe { store.player.rate() } != 0.0 {
        unsafe { store.player.pause() };
    }
}

pub(crate) fn toggle() {
    let Some(store) = player_store().as_mut() else { return };
    match store.status {
        PlayerStatus::Ready => {
            if unsafe { store.player.rate() } == 0.0 {
                unsafe { store.player.play() };
            } else {
                unsafe { store.player.pause() };
            }
        }
        PlayerStatus::Failed => quicklook_selected(),
        PlayerStatus::Loading => store.autoplay = !store.autoplay,
    }
}
