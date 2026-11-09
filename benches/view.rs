#[path = "../src/bin/app.rs"]
mod app;

#[global_allocator]
static ALLOC: rustybench::AllocProfiler = rustybench::AllocProfiler::system();

fn main() {
    if std::env::args().any(|arg| arg == "--write-corpus") {
        app::benchmarks::write_corpus_for_cli();
        return;
    }
    rustybench::main();
}
