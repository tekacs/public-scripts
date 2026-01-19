#!/usr/bin/env scriptr
---
[dependencies]
objc2 = "0.6"
objc2-foundation = "0.3"
objc2-app-kit = "0.3"
objc2-application-services = "0.3"
objc2-core-foundation = "0.3"
---

//! Opens a file in Bike and waits until that document window is closed.
//!
//! Unlike `open -a Bike -W`, this uses macOS accessibility APIs to detect
//! when the specific document is closed, rather than waiting for the app to quit.

use objc2::rc::Retained;
use objc2_app_kit::{NSRunningApplication, NSWorkspace};
use objc2_application_services::{AXError, AXUIElement};
use objc2_core_foundation::{CFArray, CFRetained, CFString, CFType};
use objc2_foundation::NSString;
use std::mem::MaybeUninit;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::time::Duration;
use std::{env, process, thread};

const BIKE_BUNDLE_ID: &str = "com.hogbaysoftware.Bike";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!("Usage: bike <file.bike>");
        eprintln!();
        eprintln!("Opens a file in Bike and waits until the document window is closed.");
        eprintln!("Uses accessibility APIs to detect document closure, unlike `open -W`");
        eprintln!("which only waits for the app to quit.");
        process::exit(if args.is_empty() { 1 } else { 0 });
    }

    let file_path = PathBuf::from(&args[0]);
    let canonical_path = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.clone());
    let file_url = format!("file://{}", canonical_path.display());

    // Remember the currently focused app
    let previous_app = get_frontmost_app();

    // Open the file in Bike
    let status = process::Command::new("open")
        .args(["-a", "Bike"])
        .args(&args)
        .status()
        .expect("failed to run open");

    if !status.success() {
        process::exit(status.code().unwrap_or(1));
    }

    // Give Bike a moment to open the document
    thread::sleep(Duration::from_millis(500));

    // Find Bike's PID
    let Some(pid) = find_bike_pid() else {
        eprintln!("Could not find Bike.app running");
        process::exit(1);
    };

    // Poll until the document is closed
    loop {
        if !is_document_open(pid, &file_url, &canonical_path) {
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }

    // Restore focus to the previously focused app
    if let Some(app) = previous_app {
        unsafe {
            app.activateWithOptions(objc2_app_kit::NSApplicationActivationOptions::empty());
        }
    }
}

fn get_frontmost_app() -> Option<Retained<NSRunningApplication>> {
    let workspace = unsafe { NSWorkspace::sharedWorkspace() };
    unsafe { workspace.frontmostApplication() }
}

fn find_bike_pid() -> Option<i32> {
    let bundle_id = NSString::from_str(BIKE_BUNDLE_ID);
    let apps = unsafe { NSRunningApplication::runningApplicationsWithBundleIdentifier(&bundle_id) };

    for app in apps.iter() {
        let pid = unsafe { app.processIdentifier() };
        if pid > 0 {
            return Some(pid);
        }
    }
    None
}

fn is_document_open(pid: i32, file_url: &str, file_path: &PathBuf) -> bool {
    let app_element = unsafe { AXUIElement::new_application(pid) };

    // Get windows
    let windows_attr = CFString::from_static_str("AXWindows");
    let Ok(windows_value) = get_attr(&app_element, &windows_attr) else {
        // App might have quit
        return false;
    };

    let Ok(windows) = windows_value.downcast::<CFArray>() else {
        return false;
    };

    let windows: CFRetained<CFArray<AXUIElement>> = unsafe { CFRetained::cast_unchecked(windows) };

    // Check each window for the document
    for window in windows.iter() {
        if let Some(doc) = ax_string_attr(&window, "AXDocument") {
            // Check if this window has our document open
            // AXDocument usually returns a file:// URL
            if doc == file_url {
                return true;
            }
            // Also check if it matches the path directly (some apps vary)
            if doc == file_path.to_string_lossy() {
                return true;
            }
            // Check if it's URL-encoded version
            if doc.contains(&percent_encode_path(file_path)) {
                return true;
            }
        }
    }

    false
}

fn get_attr(element: &AXUIElement, attr: &CFString) -> Result<CFRetained<CFType>, AXError> {
    let mut slot = MaybeUninit::<*const CFType>::uninit();
    let err = unsafe {
        element.copy_attribute_value(attr, NonNull::new_unchecked(slot.as_mut_ptr()))
    };

    if err != AXError::Success {
        return Err(err);
    }

    Ok(unsafe {
        CFRetained::<CFType>::from_raw(NonNull::new_unchecked(slot.assume_init() as *mut _))
    })
}

fn ax_string_attr(element: &AXUIElement, key: &'static str) -> Option<String> {
    let attr = CFString::from_static_str(key);
    let value = get_attr(element, &attr).ok()?;
    let s = value.downcast_ref::<CFString>()?.to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn percent_encode_path(path: &PathBuf) -> String {
    let s = path.to_string_lossy();
    s.replace(' ', "%20")
}
