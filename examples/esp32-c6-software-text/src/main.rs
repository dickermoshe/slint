// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: MIT

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel};
use slint::platform::{PlatformError, WindowAdapter};
use std::rc::Rc;

slint::include_modules!();

const DISPLAY_SIZE: slint::PhysicalSize = slint::PhysicalSize::new(320, 240);

struct EspTextPlatform {
    window: Rc<MinimalSoftwareWindow>,
}

impl slint::platform::Platform for EspTextPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
}

fn main() {
    esp_idf_svc::sys::link_patches();

    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    window.set_size(DISPLAY_SIZE);
    slint::platform::set_platform(Box::new(EspTextPlatform { window: window.clone() })).unwrap();

    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    let mut buffer =
        vec![Rgb565Pixel::default(); (DISPLAY_SIZE.width * DISPLAY_SIZE.height) as usize];

    for runtime_text in [
        "שָׁלוֹם Slint 123 — מִמָּשָׁק",
        "ASCII and עברית Hebrew · Latin 123",
        "סְלִינְט — dynamically shaped at runtime",
        "Hebrew punctuation: ״שלום״ — 123...",
    ] {
        app.set_runtime_text(runtime_text.into());
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, DISPLAY_SIZE.width as usize);
        });
    }
}
