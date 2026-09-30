use ely_gpui_component::{
    layout::on_axis,
    theme::{ActiveTheme, Radius, TextSize},
};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::db::SessionRow;

#[derive(Clone, Debug)]
pub struct MockDiffFile {
    pub path: String,
    pub status: String, // "M", "A", "D"
    pub additions: usize,
    pub deletions: usize,
}

impl BenCodeApp {
    pub fn render_diff_viewer(
        &mut self,
        _session: Option<&SessionRow>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;

        let files = vec![
            MockDiffFile {
                path: "src/main.rs".to_string(),
                status: "M".to_string(),
                additions: 42,
                deletions: 12,
            },
            MockDiffFile {
                path: "src/app.rs".to_string(),
                status: "M".to_string(),
                additions: 18,
                deletions: 5,
            },
            MockDiffFile {
                path: "Cargo.toml".to_string(),
                status: "M".to_string(),
                additions: 2,
                deletions: 1,
            },
        ];

        div()
            .flex()
            .flex_1()
            .h_full()
            .bg(colors.bg)
            // Left list of changed files
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(260.0))
                    .h_full()
                    .border_r_1()
                    .border_color(colors.border)
                    .bg(colors.surface)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .p_3()
                            .border_b_1()
                            .border_color(colors.border)
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(colors.fg)
                                    .child("CHANGED FILES (3)"),
                            ),
                    )
                    .child(
                        on_axis(div().id("diff-files-scroll"))
                            .flex_1()
                            .overflow_y_scroll()
                            .children(files.iter().enumerate().map(|(ix, f)| {
                                let is_active = ix == 0;
                                div()
                                    .id(SharedString::from(format!("diff-file-{}", f.path)))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .px_3()
                                    .py_2()
                                    .cursor_pointer()
                                    .when(is_active, |el| el.bg(colors.hover))
                                    .hover(|s| s.bg(colors.hover))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(colors.warning)
                                                    .child(f.status.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Sm))
                                                    .text_color(if is_active { colors.fg } else { colors.fg_muted })
                                                    .child(f.path.clone()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .child(
                                                div()
                                                    .text_color(colors.success)
                                                    .child(format!("+{}", f.additions)),
                                            )
                                            .child(
                                                div()
                                                    .text_color(colors.danger)
                                                    .child(format!("-{}", f.deletions)),
                                            ),
                                    )
                            })),
                    ),
            )
            // Right Unified Diff View
            .child(
                on_axis(div().id("unified-diff-scroll"))
                    .flex_1()
                    .flex_col()
                    .h_full()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .p_4()
                            .child(
                                div()
                                    .rounded(theme.radius(Radius::Md))
                                    .border_1()
                                    .border_color(colors.border)
                                    .bg(colors.surface)
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .px_4()
                                            .py_2()
                                            .border_b_1()
                                            .border_color(colors.border)
                                            .child(
                                                div()
                                                    .font_family(theme.mono_family.clone())
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(colors.fg)
                                                    .child("src/main.rs"),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .font_family(theme.mono_family.clone())
                                            .text_size(theme.text_size(TextSize::Xs))
                                            // Diff Lines
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_4()
                                                    .px_4()
                                                    .py_0p5()
                                                    .text_color(colors.fg_subtle)
                                                    .child("@@ -45,6 +45,8 @@ impl BenCodeApp {")
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_4()
                                                    .px_4()
                                                    .py_0p5()
                                                    .bg(colors.danger_subtle)
                                                    .text_color(colors.danger)
                                                    .child("-    pub fn old_render(&mut self) {")
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_4()
                                                    .px_4()
                                                    .py_0p5()
                                                    .bg(colors.success_subtle)
                                                    .text_color(colors.success)
                                                    .child("+    pub fn native_gpu_render(&mut self, window: &mut Window) {")
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_4()
                                                    .px_4()
                                                    .py_0p5()
                                                    .bg(colors.success_subtle)
                                                    .text_color(colors.success)
                                                    .child("+        // Direct Apple Metal draw call")
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_4()
                                                    .px_4()
                                                    .py_0p5()
                                                    .text_color(colors.fg)
                                                    .child("         self.render_root(window);")
                                            ),
                                    ),
                            ),
                    ),
            )
    }
}
