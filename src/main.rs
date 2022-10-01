use libremarkable::framebuffer::cgmath;
use libremarkable::framebuffer::cgmath::EuclideanSpace;
use libremarkable::framebuffer::common::*;
use libremarkable::framebuffer::storage;
use libremarkable::framebuffer::PartialRefreshMode;
use libremarkable::framebuffer::{FramebufferDraw, FramebufferIO, FramebufferRefresh};
use libremarkable::image::GenericImage;
use libremarkable::input::{gpio, multitouch, wacom, InputDevice, InputEvent};
use libremarkable::ui_extensions::element::{
    UIConstraintRefresh, UIElement, UIElementHandle, UIElementWrapper,
};
use libremarkable::{appctx, battery, image, input};
use libremarkable::{end_bench, start_bench};

#[cfg(feature = "enable-runtime-benchmarking")]
use libremarkable::stopwatch;

use atomic::Atomic;
use chrono::{DateTime, Local};
use log::info;
use once_cell::sync::Lazy;

use std::collections::VecDeque;
use std::fmt;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

#[derive(Copy, Clone, PartialEq)]
enum DrawMode {
    Draw(u32),
    Erase(u32),
}
impl DrawMode {
    fn set_size(self, new_size: u32) -> Self {
        match self {
            DrawMode::Draw(_) => DrawMode::Draw(new_size),
            DrawMode::Erase(_) => DrawMode::Erase(new_size),
        }
    }
    fn color_as_string(self) -> String {
        match self {
            DrawMode::Draw(_) => "Black",
            DrawMode::Erase(_) => "White",
        }
        .into()
    }
    fn get_size(self) -> u32 {
        match self {
            DrawMode::Draw(s) => s,
            DrawMode::Erase(s) => s,
        }
    }
}

// This region will have the following size at rest:
//   raw: 5896 kB
//   zstd: 10 kB
const FRONT_CANVAS: mxcfb_rect = mxcfb_rect {
    top: 74,
    left: 4,
    height: 896,
    width: 1396,
};
const BACK_CANVAS: mxcfb_rect = mxcfb_rect {
    top: 972,
    left: 4,
    height: 896,
    width: 1396,
};
static G_DRAW_MODE: Lazy<Atomic<DrawMode>> = Lazy::new(|| Atomic::new(DrawMode::Draw(2)));
static UNPRESS_OBSERVED: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));
static WACOM_IN_RANGE: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));
static WACOM_RUBBER_SIDE: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));
static WACOM_HISTORY: Lazy<Mutex<VecDeque<(cgmath::Point2<f32>, i32)>>> =
    Lazy::new(|| Mutex::new(VecDeque::new()));
static G_COUNTER: Lazy<Mutex<u32>> = Lazy::new(|| Mutex::new(0));
static SAVED_CANVAS: Lazy<Mutex<Option<storage::CompressedCanvasState>>> =
    Lazy::new(|| Mutex::new(None));

// ####################
// ## Button Handlers
// ####################

fn on_save_canvas(app: &mut appctx::ApplicationContext<'_>, _element: UIElementHandle) {
    start_bench!(stopwatch, save_canvas);
    let framebuffer = app.get_framebuffer_ref();
    match framebuffer.dump_region(FRONT_CANVAS) {
        Err(err) => println!("Failed to dump buffer: {0}", err),
        Ok(buff) => {
            let mut hist = SAVED_CANVAS.lock().unwrap();
            *hist = Some(storage::CompressedCanvasState::new(
                buff.as_slice(),
                FRONT_CANVAS.height,
                FRONT_CANVAS.width,
            ));
        }
    };
    end_bench!(save_canvas);
}

fn on_toggle_eraser(app: &mut appctx::ApplicationContext<'_>) {
    let (new_mode, name) = match G_DRAW_MODE.load(Ordering::Relaxed) {
        DrawMode::Erase(s) => (DrawMode::Draw(s), "Black".to_owned()),
        DrawMode::Draw(s) => (DrawMode::Erase(s), "White".to_owned()),
    };
    G_DRAW_MODE.store(new_mode, Ordering::Relaxed);
}

// ####################
// ## Miscellaneous
// ####################

/// Called on button press on rm2 or left gpio on rm1
fn quick_redraw(app: &mut appctx::ApplicationContext<'_>) {
    app.clear(false);
    app.draw_elements();
}

/// Called on button press on rm2 or middle gpio on rm1
fn full_redraw(app: &mut appctx::ApplicationContext<'_>) {
    app.clear(true);
    app.draw_elements();
}

fn change_brush_width(app: &mut appctx::ApplicationContext<'_>, delta: i32) {
    let current = G_DRAW_MODE.load(Ordering::Relaxed);
    let current_size = current.get_size() as i32;
    let proposed_size = current_size + delta;
    let new_size = if proposed_size < 1 {
        1
    } else if proposed_size > 99 {
        99
    } else {
        proposed_size
    };
    if new_size == current_size {
        return;
    }

    G_DRAW_MODE.store(current.set_size(new_size as u32), Ordering::Relaxed);
}

// ####################
// ## Input Handlers
// ####################

fn on_wacom_input(app: &mut appctx::ApplicationContext<'_>, input: input::WacomEvent) {
    match input {
        input::WacomEvent::Draw {
            position,
            pressure,
            tilt: _,
        } => {
            let mut wacom_stack = WACOM_HISTORY.lock().unwrap();

            // This is so that we can click the buttons outside the canvas region
            // normally meant to be touched with a finger using our stylus
            if !FRONT_CANVAS.contains_point(&position.cast().unwrap()) {
                wacom_stack.clear();
                if UNPRESS_OBSERVED.fetch_and(false, Ordering::Relaxed) {
                    let region = app
                        .find_active_region(position.y.round() as u16, position.x.round() as u16);
                    let element = region.map(|(region, _)| region.element.clone());
                    if let Some(element) = element {
                        (region.unwrap().0.handler)(app, element)
                    }
                }
                return;
            }

            let (mut col, mut mult) = match G_DRAW_MODE.load(Ordering::Relaxed) {
                DrawMode::Draw(s) => (color::BLACK, s),
                DrawMode::Erase(s) => (color::WHITE, s * 3),
            };
            if WACOM_RUBBER_SIDE.load(Ordering::Relaxed) {
                col = match col {
                    color::WHITE => color::BLACK,
                    _ => color::WHITE,
                };
                mult = 50; // Rough size of the rubber end
            }

            wacom_stack.push_back((position.cast().unwrap(), pressure as i32));

            while wacom_stack.len() >= 3 {
                let framebuffer = app.get_framebuffer_ref();
                let points = vec![
                    wacom_stack.pop_front().unwrap(),
                    *wacom_stack.get(0).unwrap(),
                    *wacom_stack.get(1).unwrap(),
                ];
                let radii: Vec<f32> = points
                    .iter()
                    .map(|point| ((mult as f32 * (point.1 as f32) / 2048.) / 2.0))
                    .collect();
                // calculate control points
                let start_point = points[2].0.midpoint(points[1].0);
                let ctrl_point = points[1].0;
                let end_point = points[1].0.midpoint(points[0].0);
                // calculate diameters
                let start_width = radii[2] + radii[1];
                let ctrl_width = radii[1] * 2.0;
                let end_width = radii[1] + radii[0];
                let rect = framebuffer.draw_dynamic_bezier(
                    (start_point, start_width),
                    (ctrl_point, ctrl_width),
                    (end_point, end_width),
                    10,
                    col,
                );

                framebuffer.partial_refresh(
                    &rect,
                    PartialRefreshMode::Async,
                    waveform_mode::WAVEFORM_MODE_DU,
                    display_temp::TEMP_USE_REMARKABLE_DRAW,
                    dither_mode::EPDC_FLAG_EXP1,
                    DRAWING_QUANT_BIT,
                    false,
                );
            }
        }
        input::WacomEvent::InstrumentChange { pen, state } => {
            match pen {
                // Whether the pen is in range
                input::WacomPen::ToolPen => {
                    WACOM_IN_RANGE.store(state, Ordering::Relaxed);
                    WACOM_RUBBER_SIDE.store(false, Ordering::Relaxed);
                }
                input::WacomPen::ToolRubber => {
                    WACOM_IN_RANGE.store(state, Ordering::Relaxed);
                    WACOM_RUBBER_SIDE.store(true, Ordering::Relaxed);
                }
                // Whether the pen is actually making contact
                input::WacomPen::Touch => {
                    // Stop drawing when instrument has left the vicinity of the screen
                    if !state {
                        let mut wacom_stack = WACOM_HISTORY.lock().unwrap();
                        wacom_stack.clear();
                        println!( "lift" )
                    }
                }
                _ => unreachable!(),
            }
        }
        input::WacomEvent::Hover {
            position: _,
            distance,
            tilt: _,
        } => {
            // If the pen is hovering, don't record its coordinates as the origin of the next line
            if distance > 1 {
                let mut wacom_stack = WACOM_HISTORY.lock().unwrap();
                wacom_stack.clear();
                UNPRESS_OBSERVED.store(true, Ordering::Relaxed);
            }
        }
        _ => {}
    };
}

fn on_button_press(app: &mut appctx::ApplicationContext<'_>, input: input::GPIOEvent) {
    let (btn, new_state) = match input {
        input::GPIOEvent::Press { button } => (button, true),
        input::GPIOEvent::Unpress { button } => (button, false),
        _ => return,
    };

    // Ignoring the unpressed event
    if !new_state {
        return;
    }

    // Simple but effective accidental button press filtering
    if WACOM_IN_RANGE.load(Ordering::Relaxed) {
        return;
    }

    match btn {
        input::PhysicalButton::LEFT => change_brush_width(app, -1),
        input::PhysicalButton::MIDDLE => change_brush_width(app, 1),
        input::PhysicalButton::RIGHT => on_toggle_eraser(app),
        input::PhysicalButton::POWER => {
            Command::new("systemctl")
                .arg("start")
                .arg("xochitl")
                .spawn()
                .unwrap();
            std::process::exit(0);
        }
        input::PhysicalButton::WAKEUP => {
            println!("WAKEUP button(?) pressed(?)");
        }
    };
}

fn main() {
    env_logger::init();

    // Takes callback functions as arguments
    // They are called with the event and the &mut framebuffer
    let mut app: appctx::ApplicationContext<'_> = appctx::ApplicationContext::default();

    // Alternatively we could have called `app.execute_lua("fb.clear()")`
    app.clear(true);

    // Draw the borders for the canvas region
    app.add_element(
        "frontCanvasRegion",
        UIElementWrapper {
            position: FRONT_CANVAS.top_left().cast().unwrap() + cgmath::vec2(0,0),
            refresh: UIConstraintRefresh::RefreshAndWait,
            onclick: None,
            inner: UIElement::Region {
                size: FRONT_CANVAS.size().cast().unwrap(),
                border_px: 2,
                border_color: color::BLACK,
            },
            ..Default::default()
        },
    );

    app.add_element(
        "backCanvasRegion",
        UIElementWrapper {
            position: BACK_CANVAS.top_left().cast().unwrap() + cgmath::vec2(0,0),
            refresh: UIConstraintRefresh::RefreshAndWait,
            onclick: None,
            inner: UIElement::Region {
                size: BACK_CANVAS.size().cast().unwrap(),
                border_px: 2,
                border_color: color::BLACK,
            },
            ..Default::default()
        },
    );

    // Create the top bar's time and battery labels. We can mutate these later.
    // Draw the scene
    app.draw_elements();

    // Get a &mut to the framebuffer object, exposing many convenience functions
    let appref = app.upgrade_ref();

    info!("Init complete. Beginning event dispatch...");

    // Blocking call to process events from digitizer + touchscreen + physical buttons
    app.start_event_loop(true, true, true, |ctx, evt| match evt {
        InputEvent::WacomEvent { event } => on_wacom_input(ctx, event),
       // InputEvent::MultitouchEvent { event } => on_touch_handler(ctx, event),
        InputEvent::GPIO { event } => on_button_press(ctx, event),
        _ => {}
    });
}
