//! This example is a very basic drawing application. Marker can draw on the
//! screen (without tilt or pressure sensitivity) and Marker Plus can use the
//! eraser end to erase.
//!
//! Drawing is done in the framebuffer without any caching, so it's not possible
//! to save the results to file, zoom or pan, etc. There are also no GUI
//! elements or interactivity other than the pen.
//!
//! The new event loop design makes this type of application very easy to make.

use libremarkable::appctx::ApplicationContext;
use libremarkable::framebuffer::common::{
    color, display_temp, dither_mode, waveform_mode, DRAWING_QUANT_BIT,
};
use libremarkable::framebuffer::PartialRefreshMode;
use libremarkable::framebuffer::{FramebufferDraw, FramebufferRefresh};
use libremarkable::input::{InputEvent, WacomEvent, WacomPen};

use libremarkable::framebuffer::cgmath;
use libremarkable::framebuffer::cgmath::EuclideanSpace;
use once_cell::sync::Lazy;
use std::sync::Mutex;
use std::collections::VecDeque;

static WACOM_HISTORY: Lazy<Mutex<VecDeque<(cgmath::Point2<f32>, i32)>>> =
    Lazy::new(|| Mutex::new(VecDeque::new()));

fn main() {
    let mut app = ApplicationContext::default();

    let mut tool_pen = false;
    let mut tool_rubber = false;

    app.clear(true);
    app.start_event_loop(true, false, false, |ctx, event| {
        if let InputEvent::WacomEvent { event } = event {
            match event {
                // The pen can have any number of attributes assigned to it at a
                // time. For example, when drawing with the tip of the Marker,
                // both the ToolPen and Touch attributes are applied. When
                // drawing with the eraser end of the Marker Plus, the
                // ToolRubber attribute is applied instead of ToolPen.
                //
                // The Tool attributes are mutually exclusive in practice, but
                // the protocol technically allows them to overlap. The Touch,
                // Stylus and Stylus2 events correspond to touching the display,
                // pressing the first button and pressing the second button
                // respectively. Markers don't have buttons but some Wacom pens
                // do.
                WacomEvent::InstrumentChange { pen, state } => {
                    eprintln!("pen {:?} state {}", pen, state);

                    let ptr = match pen {
                        WacomPen::ToolPen => Some(&mut tool_pen),
                        WacomPen::ToolRubber => Some(&mut tool_rubber),
                        _ => None,
                    };

                    if let Some(ptr) = ptr {
                        *ptr = state;
                    }
                }

                WacomEvent::Draw {
                    position,
                    pressure,
                    tilt: _,
                } => {
                   // eprintln!("drawing at {:?}", position);
                    let mut wacom_stack = WACOM_HISTORY.lock().unwrap();

//                    let fb = ctx.get_framebuffer_ref();

//                    let radcolor = if tool_rubber {
                        // 32 is about (>=) the physical radius of the eraser
  //                      (32, color::WHITE)
    //                } else {
      //                  (4, color::BLACK)
        //            };

                    let (mut col, mut mult) = (color::BLACK, 4);

                    wacom_stack.push_back((position.cast().unwrap(), pressure as i32));
                    while wacom_stack.len() >= 3{
                        let framebuffer = ctx.get_framebuffer_ref();
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
                            // DU mode only supports black and white colors.
                            // See the documentation of the different waveform modes
                            // for more information
                            waveform_mode::WAVEFORM_MODE_DU,
                            display_temp::TEMP_USE_REMARKABLE_DRAW,
                            dither_mode::EPDC_FLAG_EXP1,
                            DRAWING_QUANT_BIT,
                            false,
                        );
                    }
                }

                _ => {}
            }
        }
    });
}
