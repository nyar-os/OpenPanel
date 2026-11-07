//! Platform-neutral N-API surface for OpenPanel.
//!
//! Desktop GUI (`openGui` / `pumpEvents` / `closeGui`) and diagnostics stay here.
//! The WebUI listener for `open-panel server` lives in the TypeScript meta package
//! so one installable owns both the client and the server entry.

#![deny(missing_docs)]

use std::{cell::RefCell, env, path::PathBuf};

use open_panel_core::{Error as CoreError, capabilities as core_capabilities, capability_ids, platform_name};
use serde_json::{Value, json};
use wae::{
    CallContext, Error, JsValue, Result, function, optional_string_prop, register_module,
    webview::{IpcHandler, IpcOutcome, OpenOptions, WebViewError, open},
};

mod audit;
mod ops;
mod roster;

use audit::Summary;
use ops::*;
use wae::audit::Mode;

const APP_TITLE: &str = "OpenPanel";
const GUI_DIR_ENV: &str = "OPEN_PANEL_GUI";
const GUI_LOG_ENV: &str = "WAE_WEBVIEW_LOG_FILE";

thread_local! {
    static GUI: RefCell<Option<wae::webview::WebViewHost>> = const { RefCell::new(None) };
}

struct HostIpc;

impl IpcHandler for HostIpc {
    fn on_ipc(&mut self, message: &str) -> IpcOutcome {
        let Ok(request) = serde_json::from_str::<Value>(message)
        else {
            return IpcOutcome::ReplyToPage(r#"{"ok":false,"error":"invalid json"}"#.to_string());
        };
        let kind = request.get("type").and_then(Value::as_str).unwrap_or_default();
        match kind {
            "close" => IpcOutcome::CloseWindow,
            "capabilities" => match audit::run(capability_ids::CAPABILITIES, Mode::Execute, Value::Null, || {
                Ok((core_capabilities(), Summary::new(1, "capabilities")))
            }) {
                Ok(report) => IpcOutcome::ReplyToPage(reply(json!({ "ok": true, "data": report }))),
                Err(error) => IpcOutcome::ReplyToPage(reply(json!({ "ok": false, "error": error.payload() }))),
            },
            "roster.list" => match roster::list_snapshot() {
                Ok(data) => IpcOutcome::ReplyToPage(reply(json!({ "type": "roster", "ok": true, "data": data }))),
                Err(message) => IpcOutcome::ReplyToPage(reply(json!({ "type": "roster", "ok": false, "error": { "code": "roster_error", "message": message } }))),
            },
            "roster.use" => {
                let id = request.get("id").and_then(Value::as_str).unwrap_or_default();
                match roster::use_server(id) {
                    Ok(data) => IpcOutcome::ReplyToPage(reply(json!({ "type": "roster", "ok": true, "data": data }))),
                    Err(message) => IpcOutcome::ReplyToPage(reply(json!({ "type": "roster", "ok": false, "error": { "code": "roster_error", "message": message } }))),
                }
            }
            _ => IpcOutcome::ReplyToPage(reply(json!({ "ok": false, "error": "unknown ipc type" }))),
        }
    }
}

fn reply(value: Value) -> String {
    value.to_string()
}

pub(crate) fn core_error(error: CoreError) -> Error {
    let payload =
        serde_json::to_string(&error.payload()).unwrap_or_else(|_| "{\"code\":\"message\",\"message\":\"encode failed\"}".to_string());
    Error::new(format!("wae.error:{payload}"))
}

fn webview_error(error: WebViewError) -> CoreError {
    CoreError::message(error.to_string())
}

pub(crate) fn json_to_js(env: wae::Env, value: Value) -> Result<JsValue> {
    match value {
        Value::Null => env.null(),
        Value::Bool(flag) => env.boolean(flag),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                env.int64(integer)
            }
            else if let Some(unsigned) = number.as_u64() {
                env.int64(i64::try_from(unsigned).unwrap_or(i64::MAX))
            }
            else {
                env.double(number.as_f64().unwrap_or(f64::NAN))
            }
        }
        Value::String(text) => env.string(&text),
        Value::Array(items) => {
            let array = env.array_with_length(items.len())?;
            for (index, item) in items.into_iter().enumerate() {
                array.set_element(index as u32, json_to_js(env, item)?)?;
            }
            Ok(array)
        }
        Value::Object(map) => {
            let object = env.object()?;
            for (key, item) in map {
                object.set_named(&key, json_to_js(env, item)?)?;
            }
            Ok(object)
        }
    }
}

pub(crate) fn value_to_js<T: serde::Serialize>(ctx: &CallContext, value: &T) -> Result<JsValue> {
    let json = serde_json::to_value(value).map_err(|error| Error::new(error.to_string()))?;
    json_to_js(ctx.env(), json)
}

pub(crate) fn object_arg(ctx: &CallContext) -> Result<Option<JsValue>> {
    Ok(ctx.args().first().copied())
}

pub(crate) fn optional_string(obj: Option<JsValue>, name: &str) -> Result<Option<String>> {
    Ok(obj.and_then(|obj| optional_string_prop(obj, name).ok().flatten()))
}

fn resolve_gui_dir(requested: Option<String>) -> Result<String> {
    if let Some(dir) = requested
        && !dir.trim().is_empty()
    {
        return Ok(dir);
    }
    if let Ok(dir) = env::var(GUI_DIR_ENV)
        && !dir.trim().is_empty()
    {
        return Ok(dir);
    }
    let candidates = [
        PathBuf::from("packages/open-panel/dist/gui"),
        PathBuf::from("packages/open-panel-gui/dist/static"),
        PathBuf::from("dist/gui"),
        PathBuf::from("gui"),
    ];
    for candidate in candidates {
        if candidate.join("index.html").is_file() {
            return Ok(candidate.display().to_string());
        }
    }
    Err(Error::new("gui content dir not found: pass `contentDir`, set `OPEN_PANEL_GUI`, or run `pnpm run build:gui` first"))
}

fn ensure_gui_log() -> Option<PathBuf> {
    if let Some(existing) = env::var_os(GUI_LOG_ENV) {
        return Some(PathBuf::from(existing));
    }
    let dir = std::env::temp_dir().join("open-panel").join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let path = dir.join("gui.log");
    // SAFETY: N-API calls run on the Node event loop thread in this process.
    unsafe { env::set_var(GUI_LOG_ENV, &path) };
    eprintln!("[open-panel] gui.log path={}", path.display());
    Some(path)
}

fn actor_kind(value: Option<&str>) -> wae::audit::ActorKind {
    match value {
        Some(value) if value.eq_ignore_ascii_case("agent") => wae::audit::ActorKind::Agent,
        _ => wae::audit::ActorKind::User,
    }
}

fn query_from_arg(arg: Option<JsValue>) -> Result<wae::audit::Query> {
    let mut query = wae::audit::Query::default();
    let Some(arg) = arg
    else {
        return Ok(query);
    };
    query.actor_kind = optional_string_prop(arg, "kind")?.map(|value| actor_kind(Some(value.as_str())));
    query.actor_id = optional_string_prop(arg, "id")?;
    query.capability = optional_string_prop(arg, "capability")?;
    if let Some(limit) = optional_string_prop(arg, "limit")? {
        query.limit = limit.parse().ok();
    }
    Ok(query)
}

#[function]
fn version(ctx: CallContext) -> Result<JsValue> {
    let result = audit::run(capability_ids::VERSION, Mode::DryRun, Value::Null, || {
        Ok((env!("CARGO_PKG_VERSION").to_string(), Summary::new(0, "version")))
    });
    match result {
        Ok(version) => ctx.env().string(&version),
        Err(error) => Err(core_error(error)),
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BuildInfo {
    version: &'static str,
    commit: &'static str,
    built_at_unix: u64,
    target: String,
}

#[function(js_name = "buildInfo")]
fn build_info(ctx: CallContext) -> Result<JsValue> {
    let result = audit::run(capability_ids::VERSION, Mode::DryRun, Value::Null, || {
        let info = BuildInfo {
            version: env!("CARGO_PKG_VERSION"),
            commit: option_env!("WAE_APP_BUILD_COMMIT").unwrap_or("unknown"),
            built_at_unix: option_env!("WAE_APP_BUILD_EPOCH").and_then(|epoch| epoch.parse().ok()).unwrap_or(0),
            target: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        };
        Ok((info, Summary::new(1, "build info")))
    });
    match result {
        Ok(info) => value_to_js(&ctx, &info),
        Err(error) => Err(core_error(error)),
    }
}

#[function]
fn capabilities(ctx: CallContext) -> Result<JsValue> {
    let result = audit::run(capability_ids::CAPABILITIES, Mode::DryRun, Value::Null, || {
        let report = core_capabilities();
        let summary = Summary::new(report.items.len() as u64, "domain capabilities");
        Ok((report, summary))
    });
    match result {
        Ok(report) => value_to_js(&ctx, &report),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "platform")]
fn platform(ctx: CallContext) -> Result<JsValue> {
    ctx.env().string(platform_name())
}

#[function(js_name = "openGui")]
fn open_gui(ctx: CallContext) -> Result<JsValue> {
    let arg = object_arg(&ctx)?;
    let content_dir = resolve_gui_dir(optional_string(arg, "contentDir")?)?;
    let title = optional_string(arg, "title")?.unwrap_or_else(|| APP_TITLE.to_string());
    let params = json!({ "contentDir": &content_dir, "title": &title });
    let _ = ensure_gui_log();

    let result = audit::run(capability_ids::GUI_OPEN, Mode::Execute, params, || {
        let options = OpenOptions::from_content_dir(&content_dir, &title);
        let host = open(options, Box::new(HostIpc)).map_err(webview_error)?;
        let backend = host.backend_name().to_string();
        GUI.with(|slot| {
            *slot.borrow_mut() = Some(host);
        });
        Ok((backend, Summary::new(1, "gui window opened")))
    });
    match result {
        Ok(backend) => {
            let object = ctx.env().object()?;
            object.set_named("backend", ctx.env().string(&backend)?)?;
            Ok(object)
        }
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "pumpEvents")]
fn pump_events(ctx: CallContext) -> Result<JsValue> {
    let result = audit::run(capability_ids::GUI_PUMP, Mode::Execute, Value::Null, || {
        let alive = GUI.with(|slot| slot.borrow_mut().as_mut().map(|host| host.pump()).unwrap_or(false));
        Ok((alive, Summary::new(1, "gui event pump")))
    });
    match result {
        Ok(alive) => ctx.env().boolean(alive),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "closeGui")]
fn close_gui(ctx: CallContext) -> Result<JsValue> {
    let result = audit::run(capability_ids::GUI_CLOSE, Mode::Execute, Value::Null, || {
        let closed = GUI.with(|slot| {
            let mut slot = slot.borrow_mut();
            match slot.as_mut() {
                Some(host) => host.close().is_ok(),
                None => false,
            }
        });
        if closed {
            GUI.with(|slot| {
                slot.borrow_mut().take();
            });
        }
        Ok((closed, Summary::new(1, "gui window close")))
    });
    match result {
        Ok(closed) => ctx.env().boolean(closed),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "isGuiOpen")]
fn is_gui_open(ctx: CallContext) -> Result<JsValue> {
    let result = audit::run(capability_ids::GUI_STATUS, Mode::DryRun, Value::Null, || {
        let alive = GUI.with(|slot| slot.borrow().as_ref().map(|host| host.is_alive()).unwrap_or(false));
        Ok((alive, Summary::new(1, "gui status")))
    });
    match result {
        Ok(alive) => ctx.env().boolean(alive),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "configureAudit")]
fn configure_audit(ctx: CallContext) -> Result<JsValue> {
    let arg = object_arg(&ctx)?;
    let actor = wae::audit::Actor {
        kind: actor_kind(optional_string(arg, "actorKind")?.as_deref()),
        id: optional_string(arg, "actorId")?.unwrap_or_else(|| "unknown".to_string()),
        source: optional_string(arg, "actorSource")?.unwrap_or_else(|| "local".to_string()),
    };
    let session = optional_string(arg, "session")?;
    let host = optional_string(arg, "host")?;
    audit::configure(actor, session, host);
    let object = ctx.env().object()?;
    object.set_named("ok", ctx.env().boolean(true)?)?;
    Ok(object)
}

#[function(js_name = "auditQuery")]
fn audit_query(ctx: CallContext) -> Result<JsValue> {
    let filter = query_from_arg(ctx.args().first().copied())?;
    let params = json!({ "capability": filter.capability, "limit": filter.limit });
    let result = audit::run(capability_ids::AUDIT_QUERY, Mode::DryRun, params, || {
        let entries = audit::query(&filter).map_err(|error| CoreError::message(error.to_string()))?;
        let summary = Summary::new(entries.len() as u64, "audit query");
        Ok((entries, summary))
    });
    match result {
        Ok(entries) => {
            let json = serde_json::to_string(&entries).map_err(|error| Error::new(error.to_string()))?;
            ctx.env().string(&json)
        }
        Err(error) => Err(core_error(error)),
    }
}

register_module![
    version,
    build_info,
    capabilities,
    platform,
    open_gui,
    pump_events,
    close_gui,
    is_gui_open,
    configure_audit,
    audit_query,
    list_native_services,
    native_service_status,
    start_native_service,
    stop_native_service,
    restart_native_service,
    list_web_sites,
    upsert_web_site,
    remove_web_site,
    set_web_site_enabled,
    start_web_site,
    stop_web_site
];
