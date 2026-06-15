mod monitor;
mod window;

pub fn run() -> cosmic::iced::Result {
    cosmic::applet::run::<window::Window>(())
}
