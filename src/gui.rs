//! This file has the code that is responsible for the GUI.

use crate::calc::{Map, compute_etterna_difficulty, file_watcher_task};
use iced::{
    Color, Element, Event, Point, Size, Subscription, Task, Theme, event, futures::Stream, mouse::{self, Button}, widget::{center_x, center_y, column, row, scrollable, text}, window::{self, Id, Level, Position, Settings, icon},
};
use image::ImageFormat;
use minacalc_rs::{Calc, SkillsetScores};

// ============================================================================
// UI MODELS
// ============================================================================

/// Application messages.
#[derive(Clone)]
pub enum Message {
    ChangeText(String),
    UpdateCalc {
        map: Map, 
        key_count: u32,
        rate: f32
    },
    MousePressed(mouse::Button),
    MouseMoved(Point),
    MouseReleased(mouse::Button),
    WindowOpened(Id, Point)
}

/// Main application overlay state.
struct Overlay {
    calc: Calc,
    status_text: String,
    show_status: bool,
    current_map: Option<Map>,
    current_rate: Option<f32>,
    current_score: Option<SkillsetScores>,
    is_dragging: bool,
    window_id: Id,
    previous_mouse_position: Point,
    window_position: Point
}

// ============================================================================
// UI LAYER
// ============================================================================

impl Overlay {
    /// Creates a new overlay instance, initializing the Etterna calculator.
    fn new() -> Self {
        Self {
            calc: Calc::new().expect("Etterna Calc should launch"),
            status_text: String::from("Waiting for map update"),
            show_status: true,
            current_map: None,
            current_rate: None,
            current_score: None,
            is_dragging: false,
            window_id: Id::unique(),
            previous_mouse_position: Point::ORIGIN,
            window_position: Point::ORIGIN
        }
    }

    /// Processes incoming messages and updates internal state.
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ChangeText(text) => {
                self.status_text = text;
                self.show_status = true;
                Task::none()
            },
            Message::UpdateCalc {
                map,
                key_count,
                rate
            } => {
                let (normalized_rate, score) = compute_etterna_difficulty(&self.calc, &map.hit_objects, key_count, rate);
                self.show_status = false;
                self.current_map = Some(map);
                self.current_rate = Some(normalized_rate);
                self.current_score = Some(score);
                Task::none()
            },
            Message::MousePressed(mouse::Button::Left) => {
                self.is_dragging = true;
                Task::none()
            }
            Message::MouseMoved(mouse_position) => {
                if self.is_dragging {
                    let drag_offset = (mouse_position - self.previous_mouse_position)/2.0;
                    self.window_position = self.window_position + drag_offset;
                    return window::move_to(
                        self.window_id, 
                        self.window_position
                    );
                }
                self.previous_mouse_position = mouse_position;
                Task::none()
                
            }
            Message::MouseReleased(mouse::Button::Left) => {
                self.is_dragging = false;
                Task::none()
            },
            Message::WindowOpened(id, position)  => {
                self.window_id = id;
                self.window_position = position;
                Task::none()
            }
            _  => {Task::none()}
        }
    }

    /// Renders the current UI state.
    fn view(&self) -> Element<'_, Message> {
        if self.show_status {
            center_y(scrollable(center_x(text(&self.status_text))).spacing(10)).padding(10).into()
        } else {
            let map = self.current_map.as_ref().expect("map should be set when not showing status");
            let score = self.current_score.as_ref().expect("score should be set when not showing status");
            let rate = self.current_rate.expect("rate should be set when not showing status");
            center_y(scrollable(center_x(
        column![
                    column![
                        center_x(text(format!("{} - {}", map.artist, map.title)).size(20)),
                        center_x(row![text(&map.difficulty_name).size(18), text(format!("{rate:.2}")).size(18).color(color_from_rate(rate))].spacing(10))
                    ].padding(10),
                    center_x(row![
                        column![
                            text("Overall"),
                            text("Stream"),
                            text("Jumpstream"),
                            text("Handstream"),
                            text("Stamina"),
                            text("Jackspeed"),
                            text("Chordjack"),
                            text("Technical")
                        ].spacing(10),
                        column![
                            text(format!("{:.2}", score.overall)).color(color_from_difficulty(score.overall)),
                            text(format!("{:.2}", score.stream)).color(color_from_difficulty(score.stream)),
                            text(format!("{:.2}", score.jumpstream)).color(color_from_difficulty(score.jumpstream)),
                            text(format!("{:.2}", score.handstream)).color(color_from_difficulty(score.handstream)),
                            text(format!("{:.2}", score.stamina)).color(color_from_difficulty(score.stamina)),
                            text(format!("{:.2}", score.jackspeed)).color(color_from_difficulty(score.jackspeed)),
                            text(format!("{:.2}", score.chordjack)).color(color_from_difficulty(score.chordjack)),
                            text(format!("{:.2}", score.technical)).color(color_from_difficulty(score.technical))
                        ].spacing(10)
                    ].spacing(50))
                ]))).into()
        }
    }

    /// Creates a subscription that combine every other subscription.
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(vec![
            self.subscription_file_watcher(),
            self.subscription_event()
        ])
    }

    /// Creates the file watcher subscription.
    fn subscription_file_watcher(&self) -> Subscription<Message> {
        Subscription::run(Overlay::file_watcher)
    }

    /// Worker task that monitors file changes and emits messages.
    fn file_watcher() -> impl Stream<Item = Message>  {
        iced::stream::channel(100, file_watcher_task)
    }

    /// Creates an app event subscription.
    fn subscription_event(&self) -> Subscription<Message> {
        event::listen_with(|event, _status, id| {
            match event {
                Event::Mouse(iced::mouse::Event::CursorMoved { position}) => {
                    Some(Message::MouseMoved(position))
                },
                Event::Mouse(iced::mouse::Event::ButtonPressed(Button::Left)) => {
                    Some(Message::MousePressed(Button::Left))
                }
                Event::Mouse(iced::mouse::Event::ButtonReleased(Button::Left)) => {
                    Some(Message::MouseReleased(Button::Left))
                }
                Event::Window(iced::window::Event::Opened { position, size: _ }) => {
                    Some(Message::WindowOpened(id, position.expect("position should be defined at window openning")))
                }
                _ => None,
            }
        })
    }
}

// ============================================================================
// HELPER FUNCTIONS - STYLE UTILITIES
// ============================================================================

/// Maps difficulty value to a color gradient: blue -> green -> red.
///
/// - 0-20: Blue to Green
/// - 20-40: Green to Red
fn color_from_difficulty(value: f32) -> Color {
    let normalized = (value.clamp(0.0, 40.0) / 40.0).clamp(0.0, 1.0);
    gradient_color(normalized)
}

/// Maps rate value to a color gradient: blue -> green -> red.
///
/// - 0.7x: Blue
/// - 1.0x: Green
/// - 2.0x: Red
fn color_from_rate(rate: f32) -> Color {
    let normalized = ((rate.clamp(0.7, 2.0) - 0.7) / 1.3).clamp(0.0, 1.0);
    gradient_color(normalized)
}

/// Generates a color along the blue -> green -> red gradient.
///
/// Normalized value should be in [0, 1]:
/// - 0.0 = Blue (0, 0, 1)
/// - 0.5 = Green (0, 1, 0)
/// - 1.0 = Red (1, 0, 0)
fn gradient_color(normalized: f32) -> Color {
    if normalized <= 0.5 {
        // Blue to Green: [0, 0.5]
        let t = normalized / 0.5;
        Color::from_rgb(0.0, t, 1.0 - t)
    } else {
        // Green to Red: [0.5, 1.0]
        let t = (normalized - 0.5) / 0.5;
        Color::from_rgb(t, 1.0 - t, 0.0)
    }
}

/// Calculates the bottom-left corner position for the overlay window.
fn bottom_left_position(window_size: Size<f32>, monitor_size: Size<f32>) -> Point<f32> {
    Point::new(0.0, monitor_size.height - window_size.height)
}

// ============================================================================
// APPLICATION ENTRY POINT
// ============================================================================

/// Launch gui
pub fn launch_gui() -> std::result::Result<(), iced::Error> {
    let icon = icon::from_file_data(include_bytes!("icon.png"), Some(ImageFormat::Png)).unwrap();
    let window_settings = Settings {
        icon: Some(icon),
        size: Size::new(400.0, 450.0),
        position: Position::SpecificWith(bottom_left_position),
        resizable: false,
        decorations: false,
        level: Level::AlwaysOnTop,
        ..Settings::default()
    };

    return iced::application(Overlay::new, Overlay::update, Overlay::view)
        .window(window_settings)
        .title("diff-calc")
        .subscription(Overlay::subscription)
        .theme(Theme::CatppuccinMocha)
        .run();
}