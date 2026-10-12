//! The bottom status bar, and how counts and durations are written.

use super::*;

impl Humon {
    /// Bottom bar, as in Compass: total count, query time, and where you are.
    pub(crate) fn render_status_bar(&self, tab: &Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, panel) = (theme.border, theme.muted_foreground, theme.sidebar);
        let conn = self.conns.iter().find(|c| c.saved.id == tab.conn_id);
        let count = match tab.count {
            None => t!("status.counting").to_string(),
            Some(None) => t!("status.count_slow").to_string(),
            Some(Some(n)) => t!("status.documents", n = thousands(n)).to_string(),
        };
        let took = match (tab.loading, tab.took) {
            (true, _) => t!("status.running").to_string(),
            (false, Some(d)) => t!("status.took", time = format_duration(d)).to_string(),
            (false, None) => String::new(),
        };
        let filtered = !tab.applied.filter.trim().is_empty();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .px_3()
            .h(px(26.))
            .border_t_1()
            .border_color(border)
            .bg(panel)
            .text_xs()
            .text_color(muted)
            .child(count)
            .when(filtered, |d| d.child(t!("status.filtered").to_string()))
            .when(!took.is_empty(), |d| d.child("·").child(took))
            .child(div().flex_1())
            .when_some(conn, |d, c| {
                d.when_some(c.color(), |d, color| {
                    d.child(div().size(px(7.)).rounded_full().bg(color))
                })
                .child(format!("{} · {}.{}", c.saved.name, tab.db, tab.coll))
            })
    }
}

/// 1234567 -> "1.234.567", as Turkish readers expect.
pub(crate) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

pub(crate) fn format_duration(d: std::time::Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1.0 {
        "<1 ms".into()
    } else if ms < 1000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.2} s", ms / 1000.0).replace('.', ",")
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's `test` macro, which shadows `#[test]`.
    use super::{format_duration, thousands};
    use std::time::Duration;

    #[test]
    fn formats_counts_and_durations() {
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(1234), "1.234");
        assert_eq!(thousands(1234567), "1.234.567");
        assert_eq!(format_duration(Duration::from_micros(300)), "<1 ms");
        assert_eq!(format_duration(Duration::from_millis(42)), "42 ms");
        assert_eq!(format_duration(Duration::from_millis(1530)), "1,53 s");
    }
}
