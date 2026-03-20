use iced::{Element, Subscription, Task, Length};
use iced::widget::{row, container, text, shader::Shader};
use std::time::{Duration, Instant};
mod teapot;

fn main() -> iced::Result {
    iced::application("Hello Teapot", App::update, App::view)
        .subscription(App::subscription)
        .run()
}

struct App {
    scene: teapot::Scene,
    start: Instant,
}

impl Default for App {
    fn default() -> Self {
        Self { scene: teapot::Scene::new(), start: Instant::now() }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Tick(Instant),
}

impl App {
    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::Tick(now) => {
                let elapsed = now.duration_since(self.start).as_secs_f32();
                self.scene.rotation = elapsed * 0.8; // radians/second
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        row![
            // Left pane: hello world label
            container(text("Hello World").size(32))
                .center(Length::Fill),

            // Right pane: rotating teapot
            Shader::new(self.scene.clone())
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        // Fire ~60 times/second to drive rotation
        iced::time::every(Duration::from_millis(16)).map(Message::Tick)
    }
}
