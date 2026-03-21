use iced::{alignment, Element, Length, Padding, Subscription, Task};
use iced::widget::{button, column, container, mouse_area, row,
                   text, horizontal_space, vertical_space, shader::Shader, Stack};
use std::sync::Arc;
use std::time::{Duration, Instant};
use crate::teapot;

const MENUBAR_BG: iced::Color = iced::Color::from_rgb(
    0x2B as f32 / 255.0, 0x2B as f32 / 255.0, 0x2B as f32 / 255.0,
);
const MENU_BORDER: iced::Color = iced::Color::from_rgb(
    0x44 as f32 / 255.0, 0x44 as f32 / 255.0, 0x44 as f32 / 255.0,
);

fn menubar_style(_theme: &iced::Theme) -> iced::widget::container::Style {
    iced::widget::container::Style {
        background: Some(MENUBAR_BG.into()),
        ..Default::default()
    }
}

fn dropdown_style(_theme: &iced::Theme) -> iced::widget::container::Style {
    iced::widget::container::Style {
        background: Some(MENUBAR_BG.into()),
        border: iced::Border { color: MENU_BORDER, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpenMenu { None, File, Help }

pub struct App {
    pub scene: teapot::Scene,
    pub start: Instant,
    pub open_menu: OpenMenu,
    pub show_about: bool,
    /// When set, screenshots are taken each tick and sent as JPEG frames.
    pub frame_tx: Option<Arc<tokio::sync::broadcast::Sender<Vec<u8>>>>,
}

impl App {
    pub fn new() -> Self {
        Self {
            scene: teapot::Scene::new(),
            start: Instant::now(),
            open_menu: OpenMenu::None,
            show_about: false,
            frame_tx: None,
        }
    }

    pub fn with_frame_tx(tx: Arc<tokio::sync::broadcast::Sender<Vec<u8>>>) -> Self {
        Self { frame_tx: Some(tx), ..Self::new() }
    }
}

impl Default for App {
    fn default() -> Self { Self::new() }
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick(Instant),
    New,
    ToggleMenu(OpenMenu),
    CloseMenus,
    Quit,
    OpenAbout,
    CloseAbout,
    Screenshot(iced::window::Screenshot),
}

impl App {
    pub fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::New => {}
            Message::Tick(now) => {
                let elapsed = now.duration_since(self.start).as_secs_f32();
                self.scene.rotation = elapsed * 0.8;
                if self.frame_tx.is_some() {
                    return iced::window::screenshot(iced::window::Id::MAIN)
                        .map(Message::Screenshot);
                }
            }
            Message::ToggleMenu(m) => {
                self.open_menu = if self.open_menu == m { OpenMenu::None } else { m };
                self.show_about = false;
            }
            Message::CloseMenus => { self.open_menu = OpenMenu::None; }
            Message::Quit => { std::process::exit(0); }
            Message::OpenAbout => { self.show_about = true; self.open_menu = OpenMenu::None; }
            Message::CloseAbout => { self.show_about = false; }
            Message::Screenshot(shot) => {
                if let Some(ref tx) = self.frame_tx {
                    let jpeg = encode_jpeg(&shot);
                    let _ = tx.send(jpeg);
                }
            }
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let menubar = container(
            row![
                button("File").on_press(Message::ToggleMenu(OpenMenu::File)),
                button("Help").on_press(Message::ToggleMenu(OpenMenu::Help)),
            ]
            .padding(4)
            .spacing(2)
        )
        .style(menubar_style)
        .width(Length::Fill);

        let content = row![
            container(text("Hello World").size(32))
                .center(Length::Fill),
            Shader::new(self.scene.clone())
                .width(Length::Fill)
                .height(Length::Fill),
        ];

        let mut layers: Vec<Element<Message>> = vec![content.into()];

        if self.open_menu != OpenMenu::None || self.show_about {
            let dismiss_msg = if self.show_about { Message::CloseAbout } else { Message::CloseMenus };
            layers.push(
                mouse_area(
                    container(horizontal_space()).width(Length::Fill).height(Length::Fill)
                )
                .on_press(dismiss_msg)
                .into()
            );
        }

        if self.open_menu == OpenMenu::File {
            layers.push(
                container(
                    container(
                        column![
                            button("New").on_press(Message::New).width(Length::Fill),
                            button("Quit").on_press(Message::Quit).width(Length::Fill),
                        ]
                        .width(120)
                    )
                    .style(dropdown_style)
                )
                .padding(Padding { top: 0.0, left: 4.0, right: 0.0, bottom: 0.0 })
                .align_x(alignment::Horizontal::Left)
                .align_y(alignment::Vertical::Top)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
            );
        }

        if self.open_menu == OpenMenu::Help {
            layers.push(
                container(
                    container(
                        column![
                            button("About").on_press(Message::OpenAbout).width(Length::Fill)
                        ]
                        .width(120)
                    )
                    .style(dropdown_style)
                )
                .padding(Padding { top: 0.0, left: 70.0, right: 0.0, bottom: 0.0 })
                .align_x(alignment::Horizontal::Left)
                .align_y(alignment::Vertical::Top)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
            );
        }

        if self.show_about {
            layers.push(
                container(
                    container(
                        column![
                            text("About").size(20),
                            vertical_space().height(8),
                            text("Work in progress"),
                            vertical_space().height(16),
                            button("OK").on_press(Message::CloseAbout),
                        ]
                        .spacing(4)
                        .padding(24)
                    )
                    .style(|theme: &iced::Theme| {
                        let mut s = dropdown_style(theme);
                        s.border.radius = 4.0.into();
                        s
                    })
                )
                .center(Length::Fill)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
            );
        }

        column![
            menubar,
            Stack::with_children(layers).width(Length::Fill).height(Length::Fill),
        ]
        .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_millis(16)).map(Message::Tick)
    }
}

fn encode_jpeg(shot: &iced::window::Screenshot) -> Vec<u8> {
    let w = shot.size.width as usize;
    let h = shot.size.height as usize;
    let rgba: &[u8] = &shot.bytes;
    let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_EXT_RGBA);
    comp.set_size(w, h);
    comp.set_quality(82.0);
    let mut comp = comp.start_compress(Vec::new()).expect("mozjpeg start");
    comp.write_scanlines(rgba).expect("mozjpeg write");
    comp.finish().expect("mozjpeg finish")
}
