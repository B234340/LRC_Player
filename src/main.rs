mod app;
mod audio;
mod core;
mod decode;
mod ui;

use app::Application;

fn main() {
    let mut application = Application::default();
    println!("{}", application.bootstrap_summary());
    application.run_demo();
}
