use crate::types::Connection;
use anyhow::{anyhow, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Printing::{
    ClosePrinter, EndDocPrinter, EndPagePrinter, OpenPrinterW, StartDocPrinterW, StartPagePrinter,
    WritePrinter, DOC_INFO_1W, PRINTER_ACCESS_USE, PRINTER_DEFAULTSW,
};

/// PWSTR local: mismo layout que el PWSTR del crate windows
#[repr(transparent)]
struct Pwstr(*mut u16);

impl From<Vec<u16>> for Pwstr {
    fn from(mut v: Vec<u16>) -> Self {
        v.push(0);
        let p = v.as_mut_ptr();
        std::mem::forget(v);
        Pwstr(p)
    }
}

pub fn send(conn: &Connection, bytes: &[u8]) -> Result<()> {
    let printer_name = conn
        .host
        .as_deref()
        .ok_or_else(|| anyhow!("connection.host requerido"))?;

    let name_wide: Pwstr = OsStr::new(printer_name)
        .encode_wide()
        .collect::<Vec<_>>()
        .into();
    let mut handle: HANDLE = HANDLE::default();

    let defaults = PRINTER_DEFAULTSW {
        pDatatype: unsafe { std::mem::transmute_copy(&Pwstr(std::ptr::null_mut())) },
        pDevMode: std::ptr::null_mut(),
        DesiredAccess: PRINTER_ACCESS_USE,
    };

    unsafe { OpenPrinterW(PCWSTR::from_raw(name_wide.0), &mut handle, Some(&defaults)) }
        .map_err(|e| anyhow!("OpenPrinter falló '{printer_name}': {e}"))?;

    unsafe {
        let doc: Pwstr = OsStr::new("Cegel Print Bridge")
            .encode_wide()
            .collect::<Vec<_>>()
            .into();
        let raw: Pwstr = OsStr::new("RAW").encode_wide().collect::<Vec<_>>().into();
        let doc_info = DOC_INFO_1W {
            pDocName: std::mem::transmute_copy(&doc),
            pOutputFile: std::mem::transmute_copy(&Pwstr(std::ptr::null_mut())),
            pDatatype: std::mem::transmute_copy(&raw),
        };

        let job_id = StartDocPrinterW(handle, 1, &doc_info);
        if job_id == 0 {
            let _ = ClosePrinter(handle);
            return Err(anyhow!("StartDoc falló '{printer_name}'"));
        }

        let mut ok = true;
        if !StartPagePrinter(handle).as_bool() {
            ok = false;
        }
        if ok {
            let mut w: u32 = 0;
            if !WritePrinter(
                handle,
                bytes.as_ptr() as *const _,
                bytes.len() as u32,
                &mut w,
            )
            .as_bool()
            {
                ok = false;
            }
        }
        if !EndPagePrinter(handle).as_bool() {
            ok = false;
        }
        let _ = EndDocPrinter(handle);
        let _ = ClosePrinter(handle);
        if !ok {
            return Err(anyhow!("error '{printer_name}'"));
        }
    }
    log::info!("Spooler OK: '{printer_name}' ({} bytes)", bytes.len());
    Ok(())
}
