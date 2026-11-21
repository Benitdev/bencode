//! The agents' browser MCP tool handlers (`browser/mcp.rs`), running on the
//! UI thread in response to calls arriving over the browser bridge socket.
//!
//! Separated from `app/browser.rs` (tab lifecycle, picker, page events) to keep
//! each module focused.

use std::time::Duration;

use base64::Engine as _;
use gpui::{AsyncApp, Context, FutureExt as _, WeakEntity, Window};
use serde_json::Value;
use tokio::sync::oneshot;

use super::BenCodeApp;
use super::browser::{
    AGENT_SCREENSHOT_WIDTH, LOAD_TIMEOUT, PROMISE_TIMEOUT, SCRIPT_TIMEOUT, SETTLE, ZOOM_DEFAULT,
    cropped_to_rect, load_failure, outline_text, scaled_png, script_reply,
};
use super::file_pane::PaneTab;
use crate::browser::bridge::{ToolCall, ToolReply, ToolRequest};
use crate::browser::page::NO_RESULT;
use crate::browser::{agent_may_open, normalize_url, scripts, scripts_ext};

/// Max width an agent may request for `browser_screenshot`.
const MAX_SCREENSHOT_WIDTH: u32 = 2560;
/// Default visible text limit for `browser_snapshot`.
const DEFAULT_SNAPSHOT_TEXT: usize = 12_000;
const MAX_SNAPSHOT_TEXT: usize = 40_000;
/// Continuous idle duration required for `browser_wait(networkIdle: true)`.
const NETWORK_IDLE_MS: u64 = 500;

struct WaitGoal {
    text: Option<String>,
    text_gone: Option<String>,
    selector: Option<String>,
    selector_gone: Option<String>,
    url: Option<String>,
    network_idle: bool,
}

impl WaitGoal {
    fn has_condition(&self) -> bool {
        self.text.is_some()
            || self.text_gone.is_some()
            || self.selector.is_some()
            || self.selector_gone.is_some()
            || self.url.is_some()
            || self.network_idle
    }

    fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(t) = &self.text {
            parts.push(format!("text \"{t}\""));
        }
        if let Some(t) = &self.text_gone {
            parts.push(format!("text \"{t}\" gone"));
        }
        if let Some(s) = &self.selector {
            parts.push(format!("selector `{s}`"));
        }
        if let Some(s) = &self.selector_gone {
            parts.push(format!("selector `{s}` gone"));
        }
        if let Some(u) = &self.url {
            parts.push(format!("URL containing \"{u}\""));
        }
        if self.network_idle {
            parts.push("network idle".into());
        }
        parts.join(", ")
    }
}

impl BenCodeApp {
    pub(super) fn handle_tool_call(
        &mut self,
        mut call: ToolCall,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.runs.is_empty() {
            return call.answer(ToolReply::error(
                "Browser tools are only active while an agent turn is running.",
            ));
        }
        let ToolRequest { tool, args, .. } = std::mem::take(&mut call.request);
        let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::to_string);
        let flag = |key: &str| args.get(key).and_then(Value::as_bool).unwrap_or(false);

        // Tab management runs before `tool_tab`, so `list` and `new` work with 0 tabs open.
        if tool == "browser_tabs" {
            return self.handle_browser_tabs(&args, call, window, cx);
        }

        let address = match (tool == "browser_navigate").then(|| text("url")) {
            None => None,
            Some(None) => return call.answer(ToolReply::error("browser_navigate needs a url.")),
            Some(Some(url)) => {
                let url = normalize_url(&url);
                if !agent_may_open(&url) {
                    return call.answer(ToolReply::error(format!(
                        "browser_navigate opens http, https and file addresses, not {url}."
                    )));
                }
                Some(url)
            }
        };
        let (id, opened) = match self.tool_tab(address.as_deref(), window, cx) {
            Ok(tab) => tab,
            Err(why) => return call.answer(ToolReply::error(why)),
        };
        let this = cx.entity().downgrade();

        match tool.as_str() {
            "browser_navigate" => {
                let loaded = self.wait_for_load(id);
                if let Some(url) = address.filter(|_| !opened) {
                    self.browser_go(id, &url, cx);
                }
                cx.spawn(async move |_, cx| {
                    let reply = await_load(loaded, &this, id, cx).await;
                    call.answer(reply);
                })
                .detach();
            }
            "browser_history" => {
                let action = text("action");
                if !matches!(
                    action.as_deref(),
                    Some("back" | "forward" | "reload" | "hard_reload")
                ) {
                    return call.answer(ToolReply::error(
                        "action must be back, forward, reload or hard_reload.",
                    ));
                }
                let loaded = self.wait_for_load(id);
                match action.as_deref() {
                    Some("back") => self.browser_back(id, cx),
                    Some("forward") => self.browser_forward(id, cx),
                    Some("hard_reload") => self.browser_reload(id, true, cx),
                    _ => self.browser_reload(id, false, cx),
                }
                cx.spawn(async move |_, cx| {
                    cx.background_executor().timer(SETTLE).await;
                    let loading = this
                        .update(cx, |this, _| {
                            this.browser_page(id).is_some_and(|p| p.is_loading())
                        })
                        .unwrap_or(false);
                    let reply = if loading {
                        await_load(loaded, &this, id, cx).await
                    } else {
                        where_now(&this, id, "Now at", cx)
                    };
                    call.answer(reply);
                })
                .detach();
            }
            "browser_snapshot" => {
                let selector = text("selector");
                let max_text = args
                    .get("maxText")
                    .and_then(Value::as_u64)
                    .map_or(DEFAULT_SNAPSHOT_TEXT, |n| n as usize)
                    .clamp(500, MAX_SNAPSHOT_TEXT);
                let js = scripts::outline(selector.as_deref(), max_text);
                cx.spawn(async move |_, cx| {
                    let reply = match eval(&this, id, js, cx).await {
                        Ok(outline)
                            if outline.get("ok").and_then(Value::as_bool) == Some(false) =>
                        {
                            ToolReply::error(
                                outline
                                    .get("error")
                                    .and_then(Value::as_str)
                                    .unwrap_or("Snapshot failed.")
                                    .to_string(),
                            )
                        }
                        Ok(outline) => ToolReply::text(outline_text(&outline)),
                        Err(err) => ToolReply::error(err),
                    };
                    call.answer(reply);
                })
                .detach();
            }
            "browser_screenshot" => {
                let reference = args.get("ref").and_then(Value::as_u64);
                let selector = text("selector");
                let pad = args
                    .get("pad")
                    .and_then(Value::as_f64)
                    .unwrap_or(8.0)
                    .clamp(0.0, 200.0);
                let max_width = args
                    .get("maxWidth")
                    .and_then(Value::as_u64)
                    .map_or(AGENT_SCREENSHOT_WIDTH, |w| w as u32)
                    .clamp(200, MAX_SCREENSHOT_WIDTH);
                cx.notify();
                cx.spawn(async move |_, cx| {
                    let reply =
                        screenshot(&this, id, reference, selector, pad, max_width, cx).await;
                    call.answer(reply);
                })
                .detach();
            }
            "browser_click" => {
                let reference = args.get("ref").and_then(Value::as_u64);
                let selector = text("selector");
                let double_click = flag("doubleClick");
                let right_button = text("button")
                    .as_deref()
                    .is_some_and(|b| b.eq_ignore_ascii_case("right"));
                let modifiers: Vec<String> = args
                    .get("modifiers")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let js = scripts::click(
                    reference,
                    selector.as_deref(),
                    double_click,
                    right_button,
                    &modifiers,
                );
                cx.spawn(async move |_, cx| {
                    let result = eval(&this, id, js, cx).await;
                    if result.is_ok() {
                        cx.background_executor().timer(SETTLE).await;
                    }
                    let reply = script_reply(result, |value| {
                        let msg = value
                            .get("clicked")
                            .and_then(Value::as_str)
                            .unwrap_or("Clicked");
                        format!("{msg}.")
                    });
                    call.answer(reply);
                })
                .detach();
            }
            "browser_hover" => {
                let reference = args.get("ref").and_then(Value::as_u64);
                let selector = text("selector");
                let js = scripts_ext::hover(reference, selector.as_deref());
                cx.spawn(async move |_, cx| {
                    let result = eval(&this, id, js, cx).await;
                    if result.is_ok() {
                        cx.background_executor().timer(Duration::from_millis(150)).await;
                    }
                    call.answer(script_reply(result, |value| {
                        let el = value
                            .get("hovered")
                            .and_then(Value::as_str)
                            .unwrap_or("element");
                        format!("Hovered {el}.")
                    }));
                })
                .detach();
            }
            "browser_type" => {
                let reference = args.get("ref").and_then(Value::as_u64);
                let selector = text("selector");
                let typed = text("text").unwrap_or_default();
                let js = scripts::type_text(
                    reference,
                    selector.as_deref(),
                    &typed,
                    flag("submit"),
                    flag("append"),
                );
                cx.spawn(async move |_, cx| {
                    let result = eval(&this, id, js, cx).await;
                    if result.is_ok() {
                        cx.background_executor().timer(SETTLE).await;
                    }
                    call.answer(script_reply(result, |value| {
                        let el = value
                            .get("typed")
                            .and_then(Value::as_str)
                            .unwrap_or("element");
                        format!("Typed into {el}.")
                    }));
                })
                .detach();
            }
            "browser_select" => {
                let reference = args.get("ref").and_then(Value::as_u64);
                let selector = text("selector");
                let mut values: Vec<String> = args
                    .get("values")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                if values.is_empty()
                    && let Some(single) = text("value")
                {
                    values.push(single);
                }
                if values.is_empty() {
                    return call.answer(ToolReply::error(
                        "browser_select needs value or values.",
                    ));
                }
                let js = scripts_ext::select_option(reference, selector.as_deref(), &values);
                cx.spawn(async move |_, cx| {
                    let result = eval(&this, id, js, cx).await;
                    if result.is_ok() {
                        cx.background_executor().timer(SETTLE).await;
                    }
                    call.answer(script_reply(result, |value| {
                        let target = value
                            .get("target")
                            .and_then(Value::as_str)
                            .unwrap_or("select");
                        let selected: Vec<&str> = value
                            .get("selected")
                            .and_then(Value::as_array)
                            .map(|a| a.iter().filter_map(Value::as_str).collect())
                            .unwrap_or_default();
                        format!("Selected [{}] in {target}.", selected.join(", "))
                    }));
                })
                .detach();
            }
            "browser_fill_form" => {
                let Some(fields) = args.get("fields").filter(|v| v.is_array()) else {
                    return call.answer(ToolReply::error(
                        "browser_fill_form needs a fields array.",
                    ));
                };
                let js = scripts_ext::fill_form(fields, flag("submit"));
                cx.spawn(async move |_, cx| {
                    let result = eval(&this, id, js, cx).await;
                    if result.is_ok() {
                        cx.background_executor().timer(SETTLE).await;
                    }
                    call.answer(script_reply(result, |value| {
                        let lines: Vec<&str> = value
                            .get("results")
                            .and_then(Value::as_array)
                            .map(|a| a.iter().filter_map(Value::as_str).collect())
                            .unwrap_or_default();
                        if lines.is_empty() {
                            "Filled form.".into()
                        } else {
                            lines.join("\n")
                        }
                    }));
                })
                .detach();
            }
            "browser_press_key" => {
                let Some(key) = text("key") else {
                    return call.answer(ToolReply::error("browser_press_key needs a key."));
                };
                let reference = args.get("ref").and_then(Value::as_u64);
                let selector = text("selector");
                cx.spawn(async move |_, cx| {
                    let result = eval(
                        &this,
                        id,
                        scripts::press_key(&key, reference, selector.as_deref()),
                        cx,
                    )
                    .await;
                    if result.is_ok() {
                        cx.background_executor().timer(SETTLE).await;
                    }
                    call.answer(script_reply(result, |value| {
                        let effect = value.get("effect").and_then(Value::as_str).unwrap_or("");
                        format!("Pressed {key}{effect}.")
                    }));
                })
                .detach();
            }
            "browser_scroll" => {
                let reference = args.get("ref").and_then(Value::as_u64);
                let selector = text("selector");
                let direction = text("direction");
                let amount = args.get("amount").and_then(Value::as_f64);
                let into_view = flag("intoView");
                let js = scripts_ext::scroll(
                    reference,
                    selector.as_deref(),
                    direction.as_deref(),
                    amount,
                    into_view,
                );
                cx.spawn(async move |_, cx| {
                    let result = eval(&this, id, js, cx).await;
                    call.answer(script_reply(result, |value| {
                        value
                            .get("summary")
                            .and_then(Value::as_str)
                            .unwrap_or("Scrolled.")
                            .to_string()
                    }));
                })
                .detach();
            }
            "browser_drag" => {
                let from_ref = args.get("fromRef").and_then(Value::as_u64);
                let from_sel = text("fromSelector");
                let to_ref = args.get("toRef").and_then(Value::as_u64);
                let to_sel = text("toSelector");
                let js = scripts_ext::drag(
                    from_ref,
                    from_sel.as_deref(),
                    to_ref,
                    to_sel.as_deref(),
                );
                cx.spawn(async move |_, cx| {
                    let result = eval(&this, id, js, cx).await;
                    if result.is_ok() {
                        cx.background_executor().timer(SETTLE).await;
                    }
                    call.answer(script_reply(result, |value| {
                        value
                            .get("summary")
                            .and_then(Value::as_str)
                            .unwrap_or("Dragged.")
                            .to_string()
                    }));
                })
                .detach();
            }
            "browser_evaluate" => {
                let Some(expression) = text("expression") else {
                    return call.answer(ToolReply::error("browser_evaluate needs an expression."));
                };
                let eval_id = self.browser.next_eval;
                self.browser.next_eval += 1;
                cx.spawn(async move |_, cx| {
                    let reply = evaluate(&this, id, &expression, eval_id, cx).await;
                    call.answer(reply);
                })
                .detach();
            }
            "browser_inspect" => {
                let reference = args.get("ref").and_then(Value::as_u64);
                let selector = text("selector");
                let styles: Option<Vec<String>> = args
                    .get("styles")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    });
                let js =
                    scripts_ext::inspect_element(reference, selector.as_deref(), styles.as_deref());
                cx.spawn(async move |_, cx| {
                    let result = eval(&this, id, js, cx).await;
                    call.answer(script_reply(result, format_inspect));
                })
                .detach();
            }
            "browser_console" => {
                let clear = flag("clear");
                let level = text("level");
                let search = text("search");
                let limit = args.get("limit").and_then(Value::as_u64).map(|n| n as usize);
                let js = scripts::console(clear, level.as_deref(), search.as_deref(), limit);
                cx.spawn(async move |_, cx| {
                    let result = eval(&this, id, js, cx).await;
                    call.answer(script_reply(result, |value| {
                        let lines: Vec<&str> = value
                            .get("lines")
                            .and_then(Value::as_array)
                            .map(|l| l.iter().filter_map(Value::as_str).collect())
                            .unwrap_or_default();
                        if lines.is_empty() {
                            "(no console messages)".into()
                        } else {
                            lines.join("\n")
                        }
                    }));
                })
                .detach();
            }
            "browser_network" => {
                if let Some(req_id) = args.get("id").and_then(Value::as_u64) {
                    let js = scripts_ext::network_detail(req_id);
                    cx.spawn(async move |_, cx| {
                        let result = eval(&this, id, js, cx).await;
                        call.answer(script_reply(result, format_network_detail));
                    })
                    .detach();
                } else {
                    let url_f = text("url");
                    let method_f = text("method");
                    let failed_only = flag("failedOnly");
                    let include_resources = flag("includeResources");
                    let limit = args
                        .get("limit")
                        .and_then(Value::as_u64)
                        .map_or(50, |n| n as usize)
                        .clamp(1, 200);
                    let clear = flag("clear");
                    let js = scripts_ext::network_list(
                        url_f.as_deref(),
                        method_f.as_deref(),
                        failed_only,
                        include_resources,
                        limit,
                        clear,
                    );
                    cx.spawn(async move |_, cx| {
                        let result = eval(&this, id, js, cx).await;
                        call.answer(script_reply(result, format_network_list));
                    })
                    .detach();
                }
            }
            "browser_storage" => {
                let kind = text("kind").unwrap_or_else(|| "local".into());
                let action = text("action").unwrap_or_else(|| "list".into());
                let key = text("key");
                let value = text("value");
                let prefix = text("prefix");
                let include_values = flag("includeValues");
                let js = scripts_ext::storage(
                    &kind,
                    &action,
                    key.as_deref(),
                    value.as_deref(),
                    prefix.as_deref(),
                    include_values,
                );
                cx.spawn(async move |_, cx| {
                    let result = eval(&this, id, js, cx).await;
                    call.answer(script_reply(result, format_storage));
                })
                .detach();
            }
            "browser_wait" => {
                let ms = args
                    .get("ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(10_000)
                    .min(60_000);
                let goal = WaitGoal {
                    text: text("text"),
                    text_gone: text("textGone"),
                    selector: text("selector"),
                    selector_gone: text("selectorGone"),
                    url: text("url"),
                    network_idle: flag("networkIdle"),
                };
                cx.spawn(async move |_, cx| {
                    let reply = wait(&this, id, goal, Duration::from_millis(ms), cx).await;
                    call.answer(reply);
                })
                .detach();
            }
            "browser_zoom" => {
                if let Some(percent) = args.get("percent").and_then(Value::as_u64) {
                    let clamped = (percent as u16).clamp(25, 500);
                    self.set_browser_zoom(id, clamped, cx);
                } else if let Some(action) = text("action") {
                    match action.as_str() {
                        "in" => self.step_browser_zoom(id, true, cx),
                        "out" => self.step_browser_zoom(id, false, cx),
                        "reset" => self.set_browser_zoom(id, ZOOM_DEFAULT, cx),
                        "get" => {}
                        other => {
                            return call.answer(ToolReply::error(format!(
                                "Unknown zoom action {other}; use get, in, out or reset."
                            )));
                        }
                    }
                }
                let zoom = self
                    .browser
                    .tabs
                    .get(&id)
                    .map_or(ZOOM_DEFAULT, |t| t.zoom);
                let bounds = self
                    .browser_page(id)
                    .and_then(|p| p.bounds())
                    .map(|b| {
                        format!(
                            " · view {}x{}px",
                            f32::from(b.size.width).round() as i32,
                            f32::from(b.size.height).round() as i32
                        )
                    })
                    .unwrap_or_default();
                call.answer(ToolReply::text(format!("Zoom: {zoom}%{bounds}")));
            }
            "browser_dialog" => {
                let action = text("action").unwrap_or_else(|| "status".into());
                let prompt_text = text("promptText");
                let js = scripts_ext::dialog(&action, prompt_text.as_deref());
                cx.spawn(async move |_, cx| {
                    let result = eval(&this, id, js, cx).await;
                    call.answer(script_reply(result, format_dialog));
                })
                .detach();
            }
            other => call.answer(ToolReply::error(format!("Unknown browser tool {other}."))),
        }
    }

    fn handle_browser_tabs(
        &mut self,
        args: &Value,
        call: ToolCall,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("list");
        match action {
            "list" => {
                if self.browser.tabs.is_empty() {
                    return call.answer(ToolReply::text("(no open browser tabs)"));
                }
                let active = self.browser.current;
                let mut ids: Vec<u64> = self.browser.tabs.keys().copied().collect();
                ids.sort_unstable();
                let lines: Vec<String> = ids
                    .into_iter()
                    .filter_map(|id| {
                        let tab = self.browser.tabs.get(&id)?;
                        let mark = if active == Some(id) { "*" } else { " " };
                        let status = if tab.loading {
                            " [loading]"
                        } else if tab.load_failed {
                            " [failed]"
                        } else {
                            ""
                        };
                        Some(format!(
                            "{mark} tab #{id}: {} — {} (zoom {}%){status}",
                            tab.label(),
                            tab.url,
                            tab.zoom
                        ))
                    })
                    .collect();
                call.answer(ToolReply::text(lines.join("\n")));
            }
            "new" => {
                let raw_url = args.get("url").and_then(Value::as_str);
                let normalized = match raw_url {
                    Some(u) if !u.trim().is_empty() => {
                        let url = normalize_url(u);
                        if !agent_may_open(&url) {
                            return call.answer(ToolReply::error(format!(
                                "browser_tabs opens http, https and file addresses, not {url}."
                            )));
                        }
                        Some(url)
                    }
                    _ => None,
                };
                let id = self.open_browser(normalized.as_deref(), window, cx);
                if normalized.is_some() {
                    let loaded = self.wait_for_load(id);
                    let this = cx.entity().downgrade();
                    cx.spawn(async move |_, cx| {
                        let mut reply = await_load(loaded, &this, id, cx).await;
                        if reply.ok {
                            reply.text = format!("Opened tab #{id}.\n{}", reply.text);
                        }
                        call.answer(reply);
                    })
                    .detach();
                } else {
                    call.answer(ToolReply::text(format!("Opened blank tab #{id}.")));
                }
            }
            "select" => {
                let Some(id) = args.get("id").and_then(Value::as_u64) else {
                    return call.answer(ToolReply::error("browser_tabs select needs an id."));
                };
                if !self.browser.tabs.contains_key(&id) {
                    return call.answer(ToolReply::error(format!("No browser tab #{id}.")));
                }
                self.open_pane_tab(PaneTab::Browser { id }, true, cx);
                let tab = &self.browser.tabs[&id];
                call.answer(ToolReply::text(format!(
                    "Switched to tab #{id}: {} — {}",
                    tab.label(),
                    tab.url
                )));
            }
            "close" => {
                let target = args
                    .get("id")
                    .and_then(Value::as_u64)
                    .or(self.browser.current);
                let Some(id) = target else {
                    return call.answer(ToolReply::error("No browser tab to close."));
                };
                if !self.browser.tabs.contains_key(&id) {
                    return call.answer(ToolReply::error(format!("No browser tab #{id}.")));
                }
                self.drop_pane_tab(&PaneTab::Browser { id }.key(), cx);
                call.answer(ToolReply::text(format!("Closed tab #{id}.")));
            }
            other => call.answer(ToolReply::error(format!(
                "Unknown browser_tabs action {other}; use list, new, select or close."
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Async tool helpers
// ---------------------------------------------------------------------------

async fn eval(
    this: &WeakEntity<BenCodeApp>,
    id: u64,
    js: String,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let (tx, rx) = oneshot::channel();
    let started = this.update(cx, |this, _| {
        let page = this.browser_page(id)?;
        page.eval(&js, move |result| {
            if tx.send(result).is_err() {
                log::debug!("browser script answered after its caller left");
            }
        });
        Some(())
    });
    if !matches!(started, Ok(Some(()))) {
        return Err("The browser tab closed.".into());
    }
    match rx
        .with_timeout(SCRIPT_TIMEOUT, cx.background_executor())
        .await
    {
        Ok(Ok(result)) => result.map_err(|err| format!("{err:#}")),
        Ok(Err(_)) => Err("The browser tab closed.".into()),
        Err(_) => Err("The page did not answer in time (is it still loading?).".into()),
    }
}

fn where_now(this: &WeakEntity<BenCodeApp>, id: u64, verb: &str, cx: &mut AsyncApp) -> ToolReply {
    let state = this
        .update(cx, |this, _| {
            this.browser
                .tabs
                .get(&id)
                .map(|tab| (tab.title.clone(), tab.url.clone(), tab.load_failed))
        })
        .ok()
        .flatten();
    match state {
        None => ToolReply::error("The browser tab closed."),
        Some((_, url, true)) => ToolReply::error(load_failure(&url)),
        Some((title, url, false)) if title.trim().is_empty() => {
            ToolReply::text(format!("{verb} {url}"))
        }
        Some((title, url, false)) => ToolReply::text(format!("{verb}: {title}\nURL: {url}")),
    }
}

async fn await_load(
    loaded: oneshot::Receiver<bool>,
    this: &WeakEntity<BenCodeApp>,
    id: u64,
    cx: &mut AsyncApp,
) -> ToolReply {
    match loaded
        .with_timeout(LOAD_TIMEOUT, cx.background_executor())
        .await
    {
        Ok(Ok(_)) => where_now(this, id, "Loaded", cx),
        Ok(Err(_)) => ToolReply::error("The browser tab closed."),
        Err(_) => {
            let url = this
                .update(cx, |this, _| {
                    this.browser.tabs.get(&id).map(|tab| tab.url.clone())
                })
                .ok()
                .flatten()
                .unwrap_or_default();
            ToolReply::error(format!(
                "{url} did not finish loading in {}s. \
                 browser_snapshot shows what the tab has now.",
                LOAD_TIMEOUT.as_secs()
            ))
        }
    }
}

async fn screenshot(
    this: &WeakEntity<BenCodeApp>,
    id: u64,
    reference: Option<u64>,
    selector: Option<String>,
    pad: f64,
    max_width: u32,
    cx: &mut AsyncApp,
) -> ToolReply {
    // Optional element crop: scroll into view and read its bounding box first.
    let crop_target = if reference.is_some() || selector.is_some() {
        let js = scripts_ext::element_rect(reference, selector.as_deref());
        match eval(this, id, js, cx).await {
            Ok(v) if v.get("ok").and_then(Value::as_bool) == Some(true) => {
                // Give WebKit a beat to paint after scrollIntoView.
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                Some(v)
            }
            Ok(v) => {
                return ToolReply::error(
                    v.get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Element not found.")
                        .to_string(),
                );
            }
            Err(err) => return ToolReply::error(err),
        }
    } else {
        None
    };

    let shown = this
        .update(cx, |this, _| {
            this.browser_page(id).is_some_and(|p| p.is_shown())
        })
        .unwrap_or(false);
    if !shown {
        cx.background_executor()
            .timer(Duration::from_millis(300))
            .await;
    }
    let (tx, rx) = oneshot::channel();
    // When cropping an element, capture at full resolution first so the crop stays crisp.
    let capture_limit = if crop_target.is_some() {
        None
    } else {
        Some(max_width)
    };
    let started = this.update(cx, |this, _| {
        let page = this.browser_page(id)?;
        page.snapshot(capture_limit, move |png| {
            if tx.send(png).is_err() {
                log::debug!("browser snapshot after its caller left");
            }
        });
        Some(())
    });
    if !matches!(started, Ok(Some(()))) {
        return ToolReply::error("The browser tab closed.");
    }
    let png = match rx
        .with_timeout(SCRIPT_TIMEOUT, cx.background_executor())
        .await
    {
        Ok(Ok(Ok(png))) => png,
        Ok(Ok(Err(err))) => return ToolReply::error(format!("{err:#}")),
        Ok(Err(_)) => return ToolReply::error("The browser tab closed."),
        Err(_) => return ToolReply::error("WebKit took no snapshot in time."),
    };
    let label = crop_target
        .as_ref()
        .and_then(|v| v.get("described").and_then(Value::as_str))
        .map(|d| format!("Screenshot of {d}."))
        .unwrap_or_else(|| "Screenshot of the browser tab.".into());
    let encoded = cx
        .background_executor()
        .spawn(async move {
            let png = match &crop_target {
                Some(target) => cropped_to_rect(png, target, pad),
                None => png,
            };
            scaled_png(png, max_width)
                .map(|png| base64::engine::general_purpose::STANDARD.encode(png))
        })
        .await;
    match encoded {
        Ok(image) => ToolReply {
            ok: true,
            text: label,
            image: Some(image),
        },
        Err(err) => ToolReply::error(format!("The screenshot could not be read: {err:#}")),
    }
}

async fn evaluate(
    this: &WeakEntity<BenCodeApp>,
    id: u64,
    expression: &str,
    eval_id: u64,
    cx: &mut AsyncApp,
) -> ToolReply {
    let shown = |value: &Value| {
        value
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("undefined")
            .to_string()
    };
    let mut first = eval(this, id, scripts::evaluate(expression, eval_id, true), cx).await;
    if first.as_ref().is_err_and(|err| err == NO_RESULT) {
        first = eval(this, id, scripts::evaluate(expression, eval_id, false), cx).await;
    }
    let pending =
        matches!(&first, Ok(v) if v.get("pending").and_then(Value::as_bool) == Some(true));
    if !pending {
        return script_reply(first, shown);
    }
    let deadline = std::time::Instant::now() + PROMISE_TIMEOUT;
    while std::time::Instant::now() < deadline {
        cx.background_executor()
            .timer(Duration::from_millis(100))
            .await;
        let result = eval(this, id, scripts::eval_result(eval_id), cx).await;
        let still =
            matches!(&result, Ok(v) if v.get("pending").and_then(Value::as_bool) == Some(true));
        if !still {
            return script_reply(result, shown);
        }
    }
    ToolReply::error(format!(
        "The promise did not settle in {}s.",
        PROMISE_TIMEOUT.as_secs()
    ))
}

async fn wait(
    this: &WeakEntity<BenCodeApp>,
    id: u64,
    goal: WaitGoal,
    limit: Duration,
    cx: &mut AsyncApp,
) -> ToolReply {
    if !goal.has_condition() {
        cx.background_executor().timer(limit).await;
        return ToolReply::text(format!("Waited {}ms.", limit.as_millis()));
    }
    let deadline = std::time::Instant::now() + limit;
    let mut idle_since: Option<std::time::Instant> = None;
    loop {
        let js = scripts_ext::wait_check(
            goal.text.as_deref(),
            goal.text_gone.as_deref(),
            goal.selector.as_deref(),
            goal.selector_gone.as_deref(),
            goal.url.as_deref(),
            goal.network_idle,
        );
        let ready = eval(this, id, js, cx)
            .await
            .ok()
            .and_then(|v| v.get("ready").and_then(Value::as_bool))
            .unwrap_or(false);
        if ready {
            if goal.network_idle {
                let start = *idle_since.get_or_insert_with(std::time::Instant::now);
                if start.elapsed() >= Duration::from_millis(NETWORK_IDLE_MS) {
                    return ToolReply::text(format!("Condition met: {}.", goal.describe()));
                }
            } else {
                return ToolReply::text(format!("Condition met: {}.", goal.describe()));
            }
        } else {
            idle_since = None;
        }
        if std::time::Instant::now() >= deadline {
            return ToolReply::error(format!(
                "Timed out after {}ms waiting for: {}.",
                limit.as_millis(),
                goal.describe()
            ));
        }
        cx.background_executor()
            .timer(Duration::from_millis(200))
            .await;
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn format_inspect(value: &Value) -> String {
    let field = |k: &str| value.get(k).and_then(Value::as_str).unwrap_or("");
    let rect = value.get("rect").unwrap_or(&Value::Null);
    let num = |o: &Value, k: &str| o.get(k).and_then(Value::as_i64).unwrap_or(0);
    let visible = value
        .get("visible")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let in_vp = value
        .get("inViewport")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut out = format!(
        "Element: {}\nSelector: `{}`\nBox: x={}, y={}, width={}, height={} (visible={}, inViewport={})",
        field("described"),
        field("selector"),
        num(rect, "x"),
        num(rect, "y"),
        num(rect, "width"),
        num(rect, "height"),
        visible,
        in_vp,
    );
    let text = field("text");
    if !text.is_empty() {
        out.push_str(&format!("\nText: {text}"));
    }
    if let Some(attrs) = value.get("attributes").and_then(Value::as_object)
        && !attrs.is_empty()
    {
        out.push_str("\n\nAttributes:");
        for (k, v) in attrs {
            out.push_str(&format!("\n  {k}={:?}", v.as_str().unwrap_or("")));
        }
    }
    if let Some(styles) = value.get("styles").and_then(Value::as_object)
        && !styles.is_empty()
    {
        out.push_str("\n\nComputed styles:");
        for (k, v) in styles {
            out.push_str(&format!("\n  {k}: {}", v.as_str().unwrap_or("")));
        }
    }
    let html = field("html");
    if !html.is_empty() {
        out.push_str(&format!("\n\nHTML:\n```html\n{html}\n```"));
    }
    out
}

fn format_network_list(value: &Value) -> String {
    let total = value.get("total").and_then(Value::as_u64).unwrap_or(0);
    let in_flight = value.get("inFlight").and_then(Value::as_u64).unwrap_or(0);
    let lines: Vec<&str> = value
        .get("lines")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let resources: Vec<&str> = value
        .get("resources")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut out = format!(
        "Fetch/XHR requests ({} shown / {} total, {} in flight):",
        lines.len(),
        total,
        in_flight
    );
    if lines.is_empty() {
        out.push_str("\n(none)");
    } else {
        for line in &lines {
            out.push('\n');
            out.push_str(line);
        }
    }
    if !resources.is_empty() {
        out.push_str(&format!("\n\nStatic resources ({}):", resources.len()));
        for r in &resources {
            out.push('\n');
            out.push_str(r);
        }
    }
    out
}

fn format_network_detail(value: &Value) -> String {
    let Some(req) = value.get("request") else {
        return "Request not found.".into();
    };
    let field = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("");
    let id = req.get("id").and_then(Value::as_u64).unwrap_or(0);
    let status = match req.get("status").and_then(Value::as_i64) {
        Some(code) => code.to_string(),
        None => {
            let err = field("error");
            if err.is_empty() {
                "pending".into()
            } else {
                format!("error: {err}")
            }
        }
    };
    let dur = req
        .get("durationMs")
        .and_then(Value::as_i64)
        .map(|ms| format!("{ms}ms"))
        .unwrap_or_else(|| "…".into());
    let mut out = format!(
        "Request #{id} [{}]\n{} {}\nStatus: {status} · Duration: {dur}",
        field("kind"),
        field("method"),
        field("url"),
    );
    let ct = field("resType");
    if !ct.is_empty() {
        out.push_str(&format!(" · Content-Type: {ct}"));
    }
    let req_body = field("reqBody");
    if !req_body.is_empty() {
        out.push_str(&format!("\n\nRequest body:\n{req_body}"));
    }
    let res_body = field("resBody");
    if !res_body.is_empty() {
        out.push_str(&format!("\n\nResponse body:\n{res_body}"));
    }
    out
}

fn format_storage(value: &Value) -> String {
    if let Some(msg) = value.get("message").and_then(Value::as_str) {
        return msg.to_string();
    }
    let kind = value.get("kind").and_then(Value::as_str).unwrap_or("local");
    let entries = value
        .get("entries")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if entries.is_empty() {
        return format!("({kind}: empty)");
    }
    let mut lines = Vec::with_capacity(entries.len() + 1);
    let label = match kind {
        "session" => "sessionStorage",
        "cookies" => "document.cookie",
        _ => "localStorage",
    };
    lines.push(format!("{label} ({} entries):", entries.len()));
    for pair in entries {
        if let Some(arr) = pair.as_array()
            && arr.len() >= 2
        {
            lines.push(format!(
                "- {} = {}",
                arr[0].as_str().unwrap_or(""),
                arr[1].as_str().unwrap_or("")
            ));
        }
    }
    lines.join("\n")
}

fn format_dialog(value: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(msg) = value.get("message").and_then(Value::as_str) {
        parts.push(msg.to_string());
    } else if let Some(armed) = value.get("armed") {
        if armed.is_null() {
            parts.push("Dialog handler: native (not armed).".into());
        } else {
            let accept = armed
                .get("accept")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            parts.push(format!(
                "Dialog handler armed: {}.",
                if accept { "accept" } else { "dismiss" }
            ));
        }
    }
    if let Some(history) = value.get("history").and_then(Value::as_array)
        && !history.is_empty()
    {
        parts.push("Dialogs captured:".into());
        for d in history {
            let kind = d.get("type").and_then(Value::as_str).unwrap_or("dialog");
            let msg = d.get("message").and_then(Value::as_str).unwrap_or("");
            let resp = d
                .get("response")
                .map(|r| format!(" -> {r}"))
                .unwrap_or_default();
            parts.push(format!("- [{kind}] \"{msg}\"{resp}"));
        }
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn network_and_inspect_formatters_render_clean_text() {
        let list = format_network_list(&json!({
            "total": 2,
            "inFlight": 1,
            "lines": ["#1 [fetch] GET 200 18ms [application/json] /api/users"],
            "resources": ["[script] 200 12ms 4096B /assets/app.js"],
        }));
        assert!(list.contains("Fetch/XHR requests (1 shown / 2 total, 1 in flight):"));
        assert!(list.contains("#1 [fetch] GET 200 18ms"));
        assert!(list.contains("Static resources (1):"));

        let detail = format_network_detail(&json!({
            "request": {
                "id": 1,
                "kind": "fetch",
                "method": "POST",
                "url": "/api/login",
                "status": 401,
                "durationMs": 42,
                "resType": "application/json",
                "reqBody": "{\"password\":\"(redacted)\"}",
                "resBody": "{\"error\":\"bad credentials\"}",
            }
        }));
        assert!(detail.contains("Request #1 [fetch]"));
        assert!(detail.contains("Status: 401 · Duration: 42ms"));
        assert!(detail.contains("Request body:"));
        assert!(detail.contains("Response body:"));

        let inspected = format_inspect(&json!({
            "described": "button#save.primary",
            "selector": "#save",
            "rect": { "x": 10, "y": 20, "width": 80, "height": 32 },
            "visible": true,
            "inViewport": true,
            "text": "Save",
            "attributes": { "id": "save" },
            "styles": { "display": "flex", "color": "rgb(255, 255, 255)" },
            "html": "<button id=\"save\">Save</button>",
        }));
        assert!(inspected.contains("Element: button#save.primary"));
        assert!(inspected.contains("Box: x=10, y=20, width=80, height=32"));
        assert!(inspected.contains("display: flex"));
    }

    #[test]
    fn wait_goal_describes_multiple_conditions() {
        let goal = WaitGoal {
            text: Some("Dashboard".into()),
            text_gone: Some("Loading".into()),
            selector: Some("main".into()),
            selector_gone: None,
            url: Some("/app".into()),
            network_idle: true,
        };
        assert!(goal.has_condition());
        assert_eq!(
            goal.describe(),
            "text \"Dashboard\", text \"Loading\" gone, selector `main`, URL containing \"/app\", network idle"
        );
    }
}
