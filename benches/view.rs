#[path = "../src/bin/app.rs"]
mod app;

#[global_allocator]
static ALLOC: rustybench::AllocProfiler = rustybench::AllocProfiler::system();

fn main() {
    rustybench::main();
}
