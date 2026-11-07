//! Windows Service Control Manager backend (`OpenSCManager` / `OpenService`).

use std::{mem, ptr};

use windows_sys::Win32::{
    Foundation::GetLastError,
    System::Services::{
        CloseServiceHandle, ControlService, ENUM_SERVICE_STATUS_PROCESSW, EnumServicesStatusExW, OpenSCManagerW,
        OpenServiceW, QUERY_SERVICE_CONFIGW, QueryServiceConfigW, QueryServiceStatusEx, SC_ENUM_PROCESS_INFO, SC_HANDLE,
        SC_MANAGER_CONNECT, SC_MANAGER_ENUMERATE_SERVICE, SC_STATUS_PROCESS_INFO, SERVICE_CONTROL_STOP,
        SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS, SERVICE_START, SERVICE_STATE_ALL, SERVICE_STATUS,
        SERVICE_STATUS_PROCESS, SERVICE_STOP, SERVICE_WIN32, StartServiceW, SERVICE_CONTINUE_PENDING,
        SERVICE_PAUSE_PENDING, SERVICE_PAUSED, SERVICE_RUNNING, SERVICE_START_PENDING, SERVICE_STOPPED,
        SERVICE_STOP_PENDING,
    },
};

use super::{NativeServiceBackend, NativeServiceInfo, NativeServiceState, dependency, refuse_empty_name};
use crate::Result;

/// Service Control Manager backend.
pub struct ScmBackend;

impl NativeServiceBackend for ScmBackend {
    fn list(&self) -> Result<Vec<NativeServiceInfo>> {
        let manager = open_manager(SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE)?;
        let mut bytes_needed: u32 = 0;
        let mut services_returned: u32 = 0;
        let mut resume: u32 = 0;

        let probe = unsafe {
            EnumServicesStatusExW(
                manager.0,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                ptr::null_mut(),
                0,
                &mut bytes_needed,
                &mut services_returned,
                &mut resume,
                ptr::null(),
            )
        };
        if probe != 0 {
            return Ok(Vec::new());
        }
        let err = unsafe { GetLastError() };
        if err != windows_sys::Win32::Foundation::ERROR_MORE_DATA {
            return Err(dependency(format!("EnumServicesStatusExW size probe failed ({err})")));
        }

        let mut buffer = vec![0u8; bytes_needed as usize];
        resume = 0;
        let ok = unsafe {
            EnumServicesStatusExW(
                manager.0,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                buffer.as_mut_ptr(),
                bytes_needed,
                &mut bytes_needed,
                &mut services_returned,
                &mut resume,
                ptr::null(),
            )
        };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            return Err(dependency(format!("EnumServicesStatusExW failed ({err})")));
        }

        let mut out = Vec::with_capacity(services_returned as usize);
        if services_returned == 0 {
            return Ok(out);
        }
        let entries = unsafe {
            std::slice::from_raw_parts(buffer.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW, services_returned as usize)
        };
        for entry in entries {
            out.push(NativeServiceInfo {
                name: wide_ptr_to_string(entry.lpServiceName),
                display_name: wide_ptr_to_string(entry.lpDisplayName),
                state: map_state(entry.ServiceStatusProcess.dwCurrentState),
            });
        }
        out.sort_by(|left, right| left.name.to_ascii_lowercase().cmp(&right.name.to_ascii_lowercase()));
        Ok(out)
    }

    fn status(&self, name: &str) -> Result<NativeServiceInfo> {
        let name = refuse_empty_name(name)?;
        let manager = open_manager(SC_MANAGER_CONNECT)?;
        let service = open_service(manager.0, name, SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG)?;
        let status = query_status(service.0)?;
        let display = query_display_name(service.0).unwrap_or_else(|| name.to_string());
        Ok(NativeServiceInfo { name: name.to_string(), display_name: display, state: map_state(status.dwCurrentState) })
    }

    fn start(&self, name: &str) -> Result<NativeServiceInfo> {
        let name = refuse_empty_name(name)?;
        let manager = open_manager(SC_MANAGER_CONNECT)?;
        let service = open_service(manager.0, name, SERVICE_START | SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG)?;
        let status = query_status(service.0)?;
        if status.dwCurrentState == SERVICE_RUNNING {
            let display = query_display_name(service.0).unwrap_or_else(|| name.to_string());
            return Ok(NativeServiceInfo {
                name: name.to_string(),
                display_name: display,
                state: NativeServiceState::Running,
            });
        }
        let ok = unsafe { StartServiceW(service.0, 0, ptr::null()) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            return Err(dependency(format!("StartServiceW(`{name}`) failed ({err})")));
        }
        self.status(name)
    }

    fn stop(&self, name: &str) -> Result<NativeServiceInfo> {
        let name = refuse_empty_name(name)?;
        let manager = open_manager(SC_MANAGER_CONNECT)?;
        let service = open_service(manager.0, name, SERVICE_STOP | SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG)?;
        let status = query_status(service.0)?;
        if status.dwCurrentState == SERVICE_STOPPED {
            let display = query_display_name(service.0).unwrap_or_else(|| name.to_string());
            return Ok(NativeServiceInfo {
                name: name.to_string(),
                display_name: display,
                state: NativeServiceState::Stopped,
            });
        }
        let mut out_status = SERVICE_STATUS {
            dwServiceType: 0,
            dwCurrentState: 0,
            dwControlsAccepted: 0,
            dwWin32ExitCode: 0,
            dwServiceSpecificExitCode: 0,
            dwCheckPoint: 0,
            dwWaitHint: 0,
        };
        let ok = unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut out_status) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            return Err(dependency(format!("ControlService(STOP, `{name}`) failed ({err})")));
        }
        self.status(name)
    }
}

struct ServiceHandle(SC_HANDLE);

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseServiceHandle(self.0);
            }
        }
    }
}

fn open_manager(access: u32) -> Result<ServiceHandle> {
    let handle = unsafe { OpenSCManagerW(ptr::null(), ptr::null(), access) };
    if handle.is_null() {
        let err = unsafe { GetLastError() };
        return Err(dependency(format!("OpenSCManagerW failed ({err})")));
    }
    Ok(ServiceHandle(handle))
}

fn open_service(manager: SC_HANDLE, name: &str, access: u32) -> Result<ServiceHandle> {
    let wide = to_wide(name);
    let handle = unsafe { OpenServiceW(manager, wide.as_ptr(), access) };
    if handle.is_null() {
        let err = unsafe { GetLastError() };
        return Err(dependency(format!("OpenServiceW(`{name}`) failed ({err})")));
    }
    Ok(ServiceHandle(handle))
}

fn query_status(service: SC_HANDLE) -> Result<SERVICE_STATUS_PROCESS> {
    let mut bytes = 0u32;
    let mut status = SERVICE_STATUS_PROCESS {
        dwServiceType: 0,
        dwCurrentState: 0,
        dwControlsAccepted: 0,
        dwWin32ExitCode: 0,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: 0,
        dwProcessId: 0,
        dwServiceFlags: 0,
    };
    let ok = unsafe {
        QueryServiceStatusEx(
            service,
            SC_STATUS_PROCESS_INFO,
            &mut status as *mut _ as *mut u8,
            mem::size_of::<SERVICE_STATUS_PROCESS>() as u32,
            &mut bytes,
        )
    };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        return Err(dependency(format!("QueryServiceStatusEx failed ({err})")));
    }
    Ok(status)
}

fn query_display_name(service: SC_HANDLE) -> Option<String> {
    let mut needed = 0u32;
    unsafe {
        QueryServiceConfigW(service, ptr::null_mut(), 0, &mut needed);
    }
    if needed == 0 {
        return None;
    }
    let mut buffer = vec![0u8; needed as usize];
    let ok = unsafe { QueryServiceConfigW(service, buffer.as_mut_ptr() as *mut _, needed, &mut needed) };
    if ok == 0 {
        return None;
    }
    let config = unsafe { &*(buffer.as_ptr() as *const QUERY_SERVICE_CONFIGW) };
    let display = wide_ptr_to_string(config.lpDisplayName);
    if display.is_empty() { None } else { Some(display) }
}

fn map_state(code: u32) -> NativeServiceState {
    match code {
        SERVICE_STOPPED => NativeServiceState::Stopped,
        SERVICE_START_PENDING => NativeServiceState::StartPending,
        SERVICE_STOP_PENDING => NativeServiceState::StopPending,
        SERVICE_RUNNING => NativeServiceState::Running,
        SERVICE_CONTINUE_PENDING => NativeServiceState::ContinuePending,
        SERVICE_PAUSE_PENDING => NativeServiceState::PausePending,
        SERVICE_PAUSED => NativeServiceState::Paused,
        _ => NativeServiceState::Unknown,
    }
}

fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_ptr_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}
