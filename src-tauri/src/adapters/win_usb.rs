/// Adaptador USB nativo de Windows usando SetupDi + CreateFile + WriteFile.
/// No requiere libusb ni Zadig — funciona con el driver estándar de Windows.
use crate::types::Connection;
use anyhow::{anyhow, Context, Result};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows::core::GUID;
use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

/// GUID_DEVINTERFACE_USB_DEVICE: {A5DCBF10-6530-11D2-901F-00C04FB951ED}
const USB_DEVICE_GUID: GUID = GUID::from_values(
    0xA5DCBF10,
    0x6530,
    0x11D2,
    [0x90, 0x1F, 0x00, 0xC0, 0x4F, 0xB9, 0x51, 0xED],
);

pub fn send(conn: &Connection, bytes: &[u8]) -> Result<()> {
    let vendor_id = parse_hex_u16(
        "vendorId",
        conn.vendor_id
            .as_deref()
            .ok_or_else(|| anyhow!("connection.vendorId requerido para USB"))?,
    )?;
    let product_id = parse_hex_u16(
        "productId",
        conn.product_id
            .as_deref()
            .ok_or_else(|| anyhow!("connection.productId requerido para USB"))?,
    )?;

    // Buscar el device path nativo de Windows
    let device_path = find_usb_device(vendor_id, product_id)?;

    // Abrir el dispositivo con CreateFileW
    let path_wide: Vec<u16> = device_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR::from_raw(path_wide.as_ptr()),
            (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            HANDLE::default(),
        )
    }
    .map_err(|e| anyhow!("no se pudo abrir el dispositivo USB: {e:?}"))?;

    if handle == INVALID_HANDLE_VALUE {
        return Err(anyhow!(
            "no se encontró impresora USB con vendor={:04x} product={:04x}",
            vendor_id,
            product_id
        ));
    }

    // Escribir bytes
    let mut written: u32 = 0;
    unsafe {
        WriteFile(handle, Some(bytes), Some(&mut written), None)
            .map_err(|e| anyhow!("error escribiendo al dispositivo USB: {e:?}"))?;
    }

    unsafe {
        let _ = CloseHandle(handle);
    }

    log::debug!(
        "USB nativo Windows OK (vid={:04x} pid={:04x}, {} bytes)",
        vendor_id,
        product_id,
        written
    );

    Ok(())
}

fn find_usb_device(vendor_id: u16, product_id: u16) -> Result<OsString> {
    // Construir el string de búsqueda: "VID_XXXX&PID_YYYY"
    let target = format!("vid_{:04x}&pid_{:04x}", vendor_id, product_id);

    unsafe {
        let dev_info = SetupDiGetClassDevsW(
            Some(&USB_DEVICE_GUID),
            None,
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
        .map_err(|e| anyhow!("SetupDiGetClassDevsW falló: {e:?}"))?;

        let mut idx = 0u32;
        loop {
            let mut iface_data = SP_DEVICE_INTERFACE_DATA {
                cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };

            if let Err(_) =
                SetupDiEnumDeviceInterfaces(dev_info, None, &USB_DEVICE_GUID, idx, &mut iface_data)
            {
                break;
            }
            idx += 1;

            // Obtener tamaño requerido
            let mut required_size: u32 = 0;
            let mut devinfo_data = SP_DEVINFO_DATA::default();
            let _ = SetupDiGetDeviceInterfaceDetailW(
                dev_info,
                &iface_data,
                None,
                0,
                Some(&mut required_size),
                Some(&mut devinfo_data),
            );

            if required_size == 0 {
                continue;
            }

            // Asignar buffer y obtener el path
            let mut buffer: Vec<u8> = vec![0u8; required_size as usize];
            let detail = buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;

            let _ = SetupDiGetDeviceInterfaceDetailW(
                dev_info,
                &iface_data,
                Some(detail),
                required_size,
                Some(&mut required_size),
                Some(&mut devinfo_data),
            );

            let path_ptr = (*detail).DevicePath.as_ptr();
            let path_len = (0..).take_while(|&i| *path_ptr.add(i) != 0).count();
            let path_slice = std::slice::from_raw_parts(path_ptr, path_len);
            let path = OsString::from_wide(path_slice);
            let path_lower = path.to_string_lossy().to_lowercase();

            if path_lower.contains(&target) {
                return Ok(path);
            }
        }
    }

    Err(anyhow!(
        "no se encontró impresora USB con vendor={:04x} product={:04x}",
        vendor_id,
        product_id
    ))
}

fn parse_hex_u16(label: &str, value: &str) -> Result<u16> {
    let clean = value.trim().trim_start_matches("0x");
    u16::from_str_radix(clean, 16).with_context(|| format!("{label}='{value}' no es un hex válido"))
}
