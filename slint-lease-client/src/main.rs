// Demo: run a Slint UI on a DRM lease granted by the simple-graphics-controller
// daemon (@sgc), via the linuxsgc backend (slint fork, branch sgc-lease-1.17).
//
// The backend owns the whole @sgc session: connecting to the daemon, acquiring
// the card lease, and pumping the session — a revoke suspends rendering until
// the lease is re-granted (display stack rebuilt). This app never names the
// backend or SgcClient: it enables the slint `backend-linuxsgc` feature, and
// the backend selector auto-installs linuxsgc when the first window is
// created (SLINT_BACKEND=linuxsgc if several backends are enabled).
//
// Renderer chosen by cargo feature: default = software (musl static builds),
// `--features femtovg` = OpenGL over gbm/EGL (gnu dynamic builds). The GL
// renderer cannot be rebuilt in-process (its EGL/GL context dies with the lease
// fd), so the femtovg flavor exits with an error when preempted — documented
// limitation of the backend.

use std::time::Duration;

use anyhow::Context;

// The UI markup lives in ui/main.slint (compiled to Rust by build.rs);
// include_modules!() declares the generated `MainWindow` component.
slint::include_modules!();

fn main() -> anyhow::Result<()> {
    // Creating the window auto-installs the linuxsgc backend (slint feature
    // `backend-linuxsgc`): it connects to @sgc and acquires the card lease —
    // sgc or die, no daemon = no window. The app never names the backend or
    // SgcClient. (Window creation first: registering fonts needs the backend's
    // global context to exist.)
    let ui = MainWindow::new().context("creating the UI window: is the @sgc daemon running?")?;

    // Register fonts WITHOUT fontconfig: a fully static musl binary cannot
    // dlopen the board's glibc libfontconfig, so the fontique system source is
    // empty. Loading the font file into the shared collection gives the text
    // pipeline its fonts directly.
    register_font("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf")
        .context("registering DejaVuSans.ttf")?;

    // Bounce the square. Runs even while a revoke suspends rendering — the
    // backend just skips frames until the lease is re-granted, then continues.
    let ui_weak = ui.as_weak();

    // DVD bounce parameters
    let square_size = 140.0;
    let mut x_pos: f32 = 0.0;
    let mut y_pos: f32 = 0.0;
    let mut speed_x: f32 = 4.5;
    let mut speed_y: f32 = 3.75;

    // For deterministic color cycling without rand
    let mut bounce_count: u32 = 0;

    let animation = slint::Timer::default();
    animation.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(16),
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                let window = ui.window();
                let size = window.size();
                let max_x = size.width as f32 - square_size;
                let max_y = size.height as f32 - square_size;

                // Update position
                x_pos += speed_x;
                y_pos += speed_y;

                // Bounce off walls
                if x_pos <= 0.0 || x_pos >= max_x {
                    speed_x = -speed_x;
                    bounce_count += 1;
                    // Change color on bounce using a simple rainbow pattern
                    let hue = (bounce_count * 20) % 360;
                    let (r, g, b) = hsl_to_rgb(hue as f32, 0.8, 0.6);
                    let color = slint::Color::from_rgb_u8(r, g, b);
                    ui.set_square_color(color);
                }

                if y_pos <= 0.0 || y_pos >= max_y {
                    speed_y = -speed_y;
                    bounce_count += 1;
                    let hue = (bounce_count * 20) % 360;
                    let (r, g, b) = hsl_to_rgb(hue as f32, 0.8, 0.6);
                    let color = slint::Color::from_rgb_u8(r, g, b);
                    ui.set_square_color(color);
                }

                // Safety bounds
                x_pos = x_pos.clamp(0.0, max_x);
                y_pos = y_pos.clamp(0.0, max_y);

                ui.set_anim_x(x_pos);
                ui.set_anim_y(y_pos);
            }
        },
    );

    ui.run().context("event loop failed")?;
    Ok(())
}

/// Load a font file into the process-global fontique collection used by the
/// text pipeline. Returns the number of fonts registered.
fn register_font(path: &str) -> anyhow::Result<usize> {
    use slint::fontique_010::fontique;

    let bytes = std::fs::read(path).with_context(|| format!("reading font file {path}"))?;
    let blob = fontique::Blob::new(std::sync::Arc::new(bytes));
    let mut collection = slint::fontique_010::shared_collection();
    let fonts = collection.register_fonts(blob, None);
    println!("registered {} font(s) from {path}", fonts.len());
    Ok(fonts.len())
}

// Helper function for rainbow colors without the rand crate
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r, g, b) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}
