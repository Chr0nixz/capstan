//! Restricted native window adapter derived from tauri-plugin-snap-layout 1.0.9.
//! No script injection, input simulation, filesystem access, or domain services.

use tauri::{plugin::TauriPlugin, Runtime, WebviewWindow};

#[cfg(windows)]
mod windows;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("window source is not authorized")]
    Forbidden,
    #[error("invalid snap bounds")]
    Bounds,
    #[error("native window operation failed")]
    Native,
    #[error("window task ended")]
    Closed,
}

impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut value = serializer.serialize_struct("SnapError", 1)?;
        value.serialize_field(
            "code",
            match self {
                Self::Forbidden => "window_source",
                Self::Bounds => "snap_bounds",
                Self::Native => "snap_native",
                Self::Closed => "window_closed",
            },
        )?;
        value.end()
    }
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

impl Bounds {
    fn physical(self) -> Result<[i32; 4], Error> {
        if ![self.x, self.y, self.width, self.height, self.scale]
            .iter()
            .all(|value| value.is_finite())
            || self.x < 0.0
            || self.x > 32_768.0
            || !(0.0..=36.0).contains(&self.y)
            || !(1.0..=64.0).contains(&self.width)
            || !(1.0..=36.0).contains(&self.height)
            || self.y + self.height > 36.0
            || !(0.5..=8.0).contains(&self.scale)
        {
            return Err(Error::Bounds);
        }
        let left = (self.x * self.scale).round() as i32;
        let top = (self.y * self.scale).round() as i32;
        let right = ((self.x + self.width) * self.scale).round() as i32;
        let bottom = ((self.y + self.height) * self.scale).round() as i32;
        if right <= left || bottom <= top {
            return Err(Error::Bounds);
        }
        Ok([left, top, right - left, bottom - top])
    }
}

#[tauri::command]
async fn update_snap_bounds<R: Runtime>(
    webview_window: WebviewWindow<R>,
    bounds: Bounds,
) -> Result<(), Error> {
    if webview_window.label() != "main" {
        return Err(Error::Forbidden);
    }
    let rect = bounds.physical()?;
    #[cfg(windows)]
    {
        use std::sync::Arc;
        use tauri::Emitter;
        let hwnd = webview_window.hwnd().map_err(|_| Error::Native)?.0 as usize;
        let target = webview_window.clone();
        let (reply, response) = tokio::sync::oneshot::channel();
        webview_window
            .run_on_main_thread(move || {
                let result = windows::update(
                    hwnd,
                    rect,
                    Arc::new(move |event| {
                        let _ = target.emit_to("main", event, ());
                    }),
                );
                let _ = reply.send(result);
            })
            .map_err(|_| Error::Closed)?;
        response.await.map_err(|_| Error::Closed)?
    }
    #[cfg(not(windows))]
    {
        let _ = rect;
        Ok(())
    }
}

#[tauri::command]
async fn detach_snap_bounds<R: Runtime>(webview_window: WebviewWindow<R>) -> Result<(), Error> {
    if webview_window.label() != "main" {
        return Err(Error::Forbidden);
    }
    #[cfg(windows)]
    {
        let hwnd = webview_window.hwnd().map_err(|_| Error::Native)?.0 as usize;
        let (reply, response) = tokio::sync::oneshot::channel();
        webview_window
            .run_on_main_thread(move || {
                let _ = reply.send(windows::detach(hwnd));
            })
            .map_err(|_| Error::Closed)?;
        response.await.map_err(|_| Error::Closed)?
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("snap-layout")
        .invoke_handler(tauri::generate_handler![
            update_snap_bounds,
            detach_snap_bounds
        ])
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractional_dpi_and_zoom_round_edges_without_growing_the_hitbox() {
        let bounds = Bounds {
            x: 1137.2,
            y: 0.8,
            width: 46.0,
            height: 35.0,
            scale: 1.25,
        };
        assert_eq!(bounds.physical().unwrap(), [1422, 1, 57, 44]);
        assert_eq!(
            Bounds {
                scale: 1.5,
                ..bounds
            }
            .physical()
            .unwrap(),
            [1706, 1, 69, 53]
        );
    }

    #[test]
    fn rejects_nonfinite_overflow_and_body_covering_bounds() {
        let valid = Bounds {
            x: 1190.0,
            y: 0.0,
            width: 46.0,
            height: 35.0,
            scale: 1.0,
        };
        for invalid in [
            Bounds {
                x: f64::NAN,
                ..valid
            },
            Bounds {
                scale: f64::INFINITY,
                ..valid
            },
            Bounds { y: 36.0, ..valid },
            Bounds {
                width: 2000.0,
                ..valid
            },
            Bounds { x: 1e30, ..valid },
            Bounds {
                scale: 0.0,
                ..valid
            },
        ] {
            assert!(matches!(invalid.physical(), Err(Error::Bounds)));
        }
    }
}
