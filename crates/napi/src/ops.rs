//! N-API ops for native OS services and web sites.

use open_panel_core::{WebSiteInput, WebSiteKind, capability_ids, native_service, web_site};
use serde_json::{Value, json};
use wae::{CallContext, Error, JsValue, Result, function};
use wae::audit::Mode;

use crate::audit::{self, Summary};
use crate::{core_error, object_arg, optional_string, value_to_js};

fn require_string(obj: Option<JsValue>, name: &str) -> Result<String> {
    optional_string(obj, name)?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| Error::new(format!("missing string `{name}`")))
}

fn require_port(obj: Option<JsValue>) -> Result<u16> {
    let raw = require_string(obj, "port")?;
    raw.parse::<u16>().map_err(|_| Error::new("`port` must be an integer between 0 and 65535"))
}

fn optional_enabled(obj: Option<JsValue>) -> Result<bool> {
    match optional_string(obj, "enabled")?.as_deref().map(str::trim) {
        None | Some("") => Ok(true),
        Some("true") | Some("1") | Some("yes") => Ok(true),
        Some("false") | Some("0") | Some("no") => Ok(false),
        Some(other) => Err(Error::new(format!("`enabled` must be boolean-like, got `{other}`"))),
    }
}

fn site_kind(raw: Option<String>) -> Result<WebSiteKind> {
    match raw.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("static-host") => Ok(WebSiteKind::StaticHost),
        Some(other) => Err(Error::new(format!("unsupported site kind `{other}` (only `static-host`)"))),
    }
}

#[function(js_name = "listNativeServices")]
pub fn list_native_services(ctx: CallContext) -> Result<JsValue> {
    let result = audit::run(capability_ids::SERVICES, Mode::DryRun, Value::Null, || {
        let items = native_service::backend().list()?;
        let count = items.len() as u64;
        Ok((items, Summary::new(count, "native services")))
    });
    match result {
        Ok(items) => value_to_js(&ctx, &items),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "nativeServiceStatus")]
pub fn native_service_status(ctx: CallContext) -> Result<JsValue> {
    let arg = object_arg(&ctx)?;
    let name = require_string(arg, "name")?;
    let params = json!({ "name": name });
    let result = audit::run(capability_ids::SERVICES, Mode::DryRun, params, || {
        let info = native_service::backend().status(&name)?;
        Ok((info, Summary::new(1, "native service status")))
    });
    match result {
        Ok(info) => value_to_js(&ctx, &info),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "startNativeService")]
pub fn start_native_service(ctx: CallContext) -> Result<JsValue> {
    let arg = object_arg(&ctx)?;
    let name = require_string(arg, "name")?;
    let params = json!({ "name": name });
    let result = audit::run(capability_ids::SERVICES, Mode::Execute, params, || {
        let info = native_service::backend().start(&name)?;
        Ok((info, Summary::new(1, "native service start")))
    });
    match result {
        Ok(info) => value_to_js(&ctx, &info),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "stopNativeService")]
pub fn stop_native_service(ctx: CallContext) -> Result<JsValue> {
    let arg = object_arg(&ctx)?;
    let name = require_string(arg, "name")?;
    let params = json!({ "name": name });
    let result = audit::run(capability_ids::SERVICES, Mode::Execute, params, || {
        let info = native_service::backend().stop(&name)?;
        Ok((info, Summary::new(1, "native service stop")))
    });
    match result {
        Ok(info) => value_to_js(&ctx, &info),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "restartNativeService")]
pub fn restart_native_service(ctx: CallContext) -> Result<JsValue> {
    let arg = object_arg(&ctx)?;
    let name = require_string(arg, "name")?;
    let params = json!({ "name": name });
    let result = audit::run(capability_ids::SERVICES, Mode::Execute, params, || {
        let info = native_service::backend().restart(&name)?;
        Ok((info, Summary::new(1, "native service restart")))
    });
    match result {
        Ok(info) => value_to_js(&ctx, &info),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "listWebSites")]
pub fn list_web_sites(ctx: CallContext) -> Result<JsValue> {
    let result = audit::run(capability_ids::SITES, Mode::DryRun, Value::Null, || {
        let manager = web_site::shared_manager()?;
        let items = manager.list()?;
        let count = items.len() as u64;
        Ok((items, Summary::new(count, "web sites")))
    });
    match result {
        Ok(items) => value_to_js(&ctx, &items),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "upsertWebSite")]
pub fn upsert_web_site(ctx: CallContext) -> Result<JsValue> {
    let arg = object_arg(&ctx)?;
    let id = require_string(arg, "id")?;
    let domain = require_string(arg, "domain")?;
    let root_dir = require_string(arg, "rootDir")?;
    let host = optional_string(arg, "host")?.unwrap_or_else(|| "127.0.0.1".to_string());
    let port = require_port(arg)?;
    let enabled = optional_enabled(arg)?;
    let kind = site_kind(optional_string(arg, "kind")?)?;
    let input = WebSiteInput { id, domain, root_dir, host, port, enabled, kind };
    let params = serde_json::to_value(&input).unwrap_or(Value::Null);
    let result = audit::run(capability_ids::SITES, Mode::Execute, params, || {
        let manager = web_site::shared_manager()?;
        let view = manager.upsert(input)?;
        Ok((view, Summary::new(1, "web site upsert")))
    });
    match result {
        Ok(view) => value_to_js(&ctx, &view),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "removeWebSite")]
pub fn remove_web_site(ctx: CallContext) -> Result<JsValue> {
    let arg = object_arg(&ctx)?;
    let id = require_string(arg, "id")?;
    let params = json!({ "id": id });
    let result = audit::run(capability_ids::SITES, Mode::Execute, params, || {
        let manager = web_site::shared_manager()?;
        manager.remove(&id)?;
        Ok((true, Summary::new(1, "web site remove")))
    });
    match result {
        Ok(ok) => ctx.env().boolean(ok),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "setWebSiteEnabled")]
pub fn set_web_site_enabled(ctx: CallContext) -> Result<JsValue> {
    let arg = object_arg(&ctx)?;
    let id = require_string(arg, "id")?;
    let enabled = match optional_string(arg, "enabled")?.as_deref().map(str::trim) {
        Some("true") | Some("1") | Some("yes") => true,
        Some("false") | Some("0") | Some("no") => false,
        _ => return Err(Error::new("missing boolean-like `enabled`")),
    };
    let params = json!({ "id": id, "enabled": enabled });
    let result = audit::run(capability_ids::SITES, Mode::Execute, params, || {
        let manager = web_site::shared_manager()?;
        let view = manager.set_enabled(&id, enabled)?;
        Ok((view, Summary::new(1, "web site enable")))
    });
    match result {
        Ok(view) => value_to_js(&ctx, &view),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "startWebSite")]
pub fn start_web_site(ctx: CallContext) -> Result<JsValue> {
    let arg = object_arg(&ctx)?;
    let id = require_string(arg, "id")?;
    let params = json!({ "id": id });
    let result = audit::run(capability_ids::SITES, Mode::Execute, params, || {
        let manager = web_site::shared_manager()?;
        let view = manager.start(&id)?;
        Ok((view, Summary::new(1, "web site start")))
    });
    match result {
        Ok(view) => value_to_js(&ctx, &view),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "stopWebSite")]
pub fn stop_web_site(ctx: CallContext) -> Result<JsValue> {
    let arg = object_arg(&ctx)?;
    let id = require_string(arg, "id")?;
    let params = json!({ "id": id });
    let result = audit::run(capability_ids::SITES, Mode::Execute, params, || {
        let manager = web_site::shared_manager()?;
        let view = manager.stop(&id)?;
        Ok((view, Summary::new(1, "web site stop")))
    });
    match result {
        Ok(view) => value_to_js(&ctx, &view),
        Err(error) => Err(core_error(error)),
    }
}
