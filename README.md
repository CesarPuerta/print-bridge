# Cegel Print Bridge

Daemon local (escritorio) que recibe trabajos ESC/POS enviados por la web app de Cegel (`www.cegel.app`) y los envía a la impresora térmica conectada al equipo. También acciona el cajón monedero (drawer kick).

Construido con **Tauri 2 + Rust**. Ligero (~10 MB), corre en segundo plano en la bandeja del sistema y arranca automáticamente con el equipo.

---

## Tabla de contenidos

- [Arquitectura](#arquitectura)
- [Endpoints HTTP](#endpoints-http)
- [Instalación para usuarios finales (cajeros)](#instalación-para-usuarios-finales-cajeros)
  - [macOS](#macos)
  - [Windows](#windows)
  - [Linux](#linux)
- [Configuración inicial / vinculación](#configuración-inicial--vinculación)
- [Setup de desarrollo](#setup-de-desarrollo)
- [Build & distribución sin pagar certificados](#build--distribución-sin-pagar-certificados)
- [Convenciones de código](#convenciones-de-código)

---

## Arquitectura

```
Web app (www.cegel.app)  ─►  HTTP POST http://127.0.0.1:9101/print
                                       │
                                       ▼
                          ┌────────────────────────┐
                          │  Cegel Print Bridge    │  (esta app, Rust)
                          │  ─ axum HTTP server    │
                          │  ─ adapters/usb,tcp,   │
                          │     serial             │
                          └───────────┬────────────┘
                                      ▼
                          Impresora térmica ESC/POS
```

El backend de Cegel genera los bytes ESC/POS (incluyendo QR DIAN, corte y pulso del cajón) y los devuelve en `base64`. El bridge sólo es transporte.

---

## Endpoints HTTP

Expuestos en `127.0.0.1:9101` (loopback únicamente, jamás expuesto a la red):

| Método | Ruta           | Descripción                                                                |
| ------ | -------------- | -------------------------------------------------------------------------- |
| `GET`  | `/health`      | Devuelve `{ ok, version, paired, businessId, deviceId }`.                  |
| `POST` | `/print`       | Imprime un ticket. Body: `{ bytesBase64, connection, printerId, label? }`. |
| `POST` | `/drawer-kick` | Abre el cajón. Mismo body que `/print` (bytes = `ESC p m`).                |

CORS permite por defecto `https://www.cegel.app`, `https://cegel.app`, `http://localhost:5173` y `http://127.0.0.1:5173`.

Todas las peticiones a `/print` y `/drawer-kick` deben enviar el header `X-Cegel-Business: <businessId>`. Si el bridge está vinculado a un negocio distinto, responde **403**. Esto evita que un cajero logueado en otra cuenta dispare la impresora del local equivocado.

### Forma de `connection`

```json
{ "type": "network", "host": "192.168.1.50", "port": 9100 }
{ "type": "usb", "vendorId": "04b8", "productId": "0e15" }
{ "type": "serial", "path": "/dev/tty.usbserial", "baudRate": 9600 }
```

---

## Instalación para usuarios finales (cajeros)

El bridge se instala **una sola vez en cada equipo** que tenga impresora térmica conectada. No se instala en celulares ni en computadores que sólo usan la web sin imprimir.

Descargar siempre el instalador desde `https://www.cegel.app/descargas` (o el link directo que entregue soporte).

### macOS

1. Descargar `Cegel Print Bridge_0.x.x_aarch64.dmg` desde [github.com/CesarPuerta/print-bridge/releases](https://github.com/CesarPuerta/print-bridge/releases)
2. Doble clic en el DMG → arrastrar **Cegel Print Bridge** a **Aplicaciones**
3. Si macOS lo bloquea con _"está dañado y no puede abrirse"_, ejecutar en Terminal:
   ```bash
   xattr -cr /Applications/Cegel\ Print\ Bridge.app
   ```
4. Primera ejecución: **clic derecho** sobre la app → **Abrir** → en el diálogo, **Abrir**
5. Conceder permisos de red local cuando macOS pregunte (requerido para `127.0.0.1:9101`)
6. Verificar: el ícono aparece en la barra de menús → "Estado: Activo"

### Windows

1. Descargar `Cegel Print Bridge_0.x.x_x64-setup.exe`
2. Doble clic → si SmartScreen bloquea: **Más información → Ejecutar de todos modos**
3. Seguir el instalador (NSIS), marcar "Iniciar con Windows"
4. Verificar: ícono en la bandeja del sistema (junto al reloj) → "Estado: Activo"

> ⚠️ **Si la app muestra "Servidor no responde" en Windows**, ver [Troubleshooting Windows](#troubleshooting-windows).

### Linux

1. Descargar `cegel-print-bridge_*.AppImage`
2. `chmod +x cegel-print-bridge*.AppImage && ./cegel-print-bridge*.AppImage`
3. Para acceso USB sin sudo: `sudo usermod -aG plugdev,dialout $USER` (cerrar sesión)

---

## Obtener Vendor ID y Product ID de la impresora USB

Al crear la impresora en `www.cegel.app/manager/printers` con tipo **USB**, necesitas estos dos valores hexadecimales.

### macOS

```bash
# Conectar la impresora por USB y ejecutar:
ioreg -r -c IOUSBDevice 2>&1 | grep -E "\"USB Product Name\"|\"idVendor\"|\"idProduct\""

# Buscar "USB Product Name" con tu modelo de impresora (ej: "Printer-80", "TM-T20")
# Tomar los valores decimales de idVendor e idProduct.

# Convertir decimal → hex con:
printf "Vendor:  8137 → 0x%04x\nProduct: 8214 → 0x%04x\n"
```

### Windows

```powershell
# PowerShell como Administrador — detecta CUALQUIER impresora USB (POS, térmica, etc.):
Get-PnpDevice -PresentOnly -Class USB | Where-Object {
  $_.FriendlyName -like "*Printer*" -or
  $_.FriendlyName -like "*POS*" -or
  $_.FriendlyName -like "*Thermal*"
} | Select-Object FriendlyName, InstanceId,
  @{n='VendorID';e={[regex]::Match($_.InstanceId, 'VID_([0-9A-F]{4})').Groups[1].Value}},
  @{n='ProductID';e={[regex]::Match($_.InstanceId, 'PID_([0-9A-F]{4})').Groups[1].Value}} |
  Format-Table -AutoSize

# Alternativa manual: Device Manager → Universal Serial Bus devices →
# clic derecho en la impresora → Properties → Details → Hardware Ids
# Buscar: USB\VID_04B8&PID_0E03 → vendorId: 04b8, productId: 0e03
```

### Valores comunes de Epson

| Modelo    | Vendor ID | Product ID |
| --------- | --------- | ---------- |
| TM-T20    | `04b8`    | `0e03`     |
| TM-T20II  | `04b8`    | `0e28`     |
| TM-T20III | `04b8`    | `0e47`     |
| TM-T88V   | `04b8`    | `0202`     |
| TM-U220   | `04b8`    | `0202`     |

---

## Configuración inicial / vinculación

1. Instala el bridge en el equipo que tiene la impresora conectada.
2. Abre la ventana del bridge (clic en el ícono de la bandeja → **Mostrar ventana**).
3. Haz clic en **Vincular equipo…** → se genera un código de 64 caracteres.
4. Abre `https://www.cegel.app/manager/devices` → **Vincular equipo** → pega el código.
5. El bridge detecta la vinculación automáticamente y queda listo para imprimir.

El estado se persiste en `~/.cegel/bridge.json`:

```json
{
  "port": 9101,
  "allowed_origins": ["https://www.cegel.app", "https://cegel.app"],
  "paired_business_id": "69f7...",
  "device_token": "<hex 64>",
  "device_id": "<uuid>",
  "cegel_api_base": "https://api.cegel.app"
}
```

Para desvincular: ventana del bridge → doble clic en **Desvincular**.

---

## Setup de desarrollo

### Requisitos

- **Rust** 1.78+ (`curl https://sh.rustup.rs -sSf | sh`)
- **Node** 20+
- macOS: Xcode Command Line Tools (`xcode-select --install`)
- Linux: `sudo apt install libusb-1.0-0-dev libudev-dev libssl-dev libwebkit2gtk-4.1-dev`
- Windows: WebView2 (incluido en Win11) + MSVC Build Tools

### Pasos

```bash
git clone https://github.com/CesarPuerta/print-bridge.git
cd print-bridge
npm install            # instala Tauri CLI + tooling JS (lint, prettier, husky)
npm run dev            # ventana + servidor en :9101 con hot reload
```

Husky se instala solo en `npm install` (excepto cuando `NODE_ENV=production`).

### Comandos útiles

```bash
npm run lint           # ESLint sobre src/
npm run prettier       # check de formato
npm run format         # prettier + eslint --fix
npm run rust:fmt       # cargo fmt
npm run rust:lint      # cargo clippy
```

---

## Build & distribución sin pagar certificados

**Sí, funciona perfecto sin pagar nada.** El bridge no necesita firma para correr; los usuarios sólo ven una advertencia la primera vez. Los costos de signing son opcionales y sólo eliminan ese diálogo inicial.

### Build local sin firma (gratis)

```bash
# macOS (genera DMG ad-hoc, no firmado)
npm run build:unsigned

# Windows / Linux (no requieren signing por defecto)
npm run build
```

Salidas en `src-tauri/target/release/bundle/`:

- macOS: `dmg/Cegel-Print-Bridge_*.dmg`
- Windows: `nsis/Cegel-Print-Bridge_*-setup.exe` y `msi/*.msi`
- Linux: `appimage/*.AppImage` y `deb/*.deb`

### Lo que ven los usuarios con el binario sin firmar

| OS      | Bloqueo inicial                                    | Solución del usuario                                                |
| ------- | -------------------------------------------------- | ------------------------------------------------------------------- |
| macOS   | “No se puede abrir, desarrollador no identificado” | Clic derecho → Abrir → Abrir. **Solo la primera vez.**              |
| Windows | SmartScreen “PC protegido por Windows”             | Más información → Ejecutar de todos modos. **Solo la primera vez.** |
| Linux   | Ninguno                                            | —                                                                   |

Documentar esto en la página de descargas evita el 90 % de los tickets de soporte.

### Cuándo conviene pagar (opcional, no obligatorio)

| Tipo de firma           | Costo aprox.      | Beneficio                                             |
| ----------------------- | ----------------- | ----------------------------------------------------- |
| Apple Developer ID      | USD 99 / año      | Quita la advertencia en macOS + notarización          |
| Windows EV Code Signing | USD 200–400 / año | Quita SmartScreen inmediatamente                      |
| Windows OV Code Signing | USD 70–200 / año  | Quita SmartScreen tras ~10 instalaciones (reputación) |

**Recomendación:** lanzar sin firmar, documentar el “clic derecho → Abrir” en `/descargas`. Comprar firma sólo cuando el volumen de soporte lo justifique.

### Auto-update (gratis con GitHub Releases)

Tauri Updater puede chequear releases de un repositorio GitHub público y descargar la nueva versión automáticamente. Configurar en `src-tauri/tauri.conf.json` → `plugins.updater.endpoints` (pendiente entrega 5).

### Release final e instaladores

1. Actualiza la versión en `package.json` y `src-tauri/tauri.conf.json`.
2. Ejecuta una prueba local: `npm run dev`.
3. Genera los bundles: `npm run build:unsigned` en macOS y `npm run build` en Windows/Linux.
4. Revisa `src-tauri/target/release/bundle/`.
5. Etiqueta la versión: `git tag v0.1.1 && git push origin v0.1.1`.
6. GitHub Actions publica los instaladores desde `.github/workflows/release.yml`.

---

## Convenciones de código

Mismas reglas que `soluciona-api` y `soluciona-web`:

- **Commits**: [Conventional Commits](https://www.conventionalcommits.org/). Validado por `commitlint` en `commit-msg` hook.
  - Tipos: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `chore`, `ci`, `build`, `revert`
  - Ejemplo: `feat(pairing): agregar polling con timeout de 10 min`
- **Formato JS/JSON/MD**: Prettier (auto en `pre-commit` vía `lint-staged`).
- **Lint JS**: ESLint flat config.
- **Formato Rust**: `cargo fmt` (auto en `pre-commit` para `src-tauri/**/*.rs`).
- **Lint Rust**: `cargo clippy -- -D warnings` (validado en `pre-push`).

### Hooks de Git (Husky)

| Hook         | Acción                                           |
| ------------ | ------------------------------------------------ |
| `pre-commit` | `lint-staged` → prettier + eslint + cargo fmt    |
| `commit-msg` | `commitlint` valida el mensaje                   |
| `pre-push`   | `cargo fmt --check` + `cargo clippy -D warnings` |

---

## Estructura

```
soluciona-print-bridge/
├── package.json
├── commitlint.config.cjs
├── .prettierrc.cjs / .prettierignore
├── eslint.config.mjs
├── .editorconfig
├── .husky/                    # pre-commit, commit-msg, pre-push
├── src/                       # frontend de la ventana (vanilla HTML/JS)
│   ├── index.html
│   ├── main.js
│   └── styles.css
└── src-tauri/
    ├── Cargo.toml
    ├── rustfmt.toml
    ├── tauri.conf.json
    ├── build.rs
    ├── capabilities/default.json
    └── src/
        ├── main.rs            # entry
        ├── lib.rs             # setup Tauri + tray + spawn server + autostart
        ├── server.rs          # axum: /health, /print, /drawer-kick + enforce_business
        ├── pairing.rs         # flujo de pairing + heartbeat
        ├── types.rs           # PrintJob, Connection, respuestas
        ├── config.rs          # ~/.cegel/bridge.json
        └── adapters/
            ├── mod.rs
            ├── tcp.rs         # red 9100
            ├── usb.rs         # rusb / libusb bulk OUT
            └── serial.rs      # serialport-rs
```

---

## Troubleshooting Windows

### "Servidor no responde" en la app de escritorio (Windows)

Esto ocurre cuando el frontend de Tauri (WebView2) no puede alcanzar el endpoint `/health`.
El servidor **ya está corriendo** (confirmado con `curl`), pero WebView2 a veces usa
IPv6 (`::1`) para `localhost` mientras el bridge solo escuchaba en IPv4 (`127.0.0.1`).

**A partir de v0.2.6**, el bridge escucha en ambas interfaces loopback (`127.0.0.1` y `::1`),
lo que resuelve el problema para la mayoría de los casos. Si aún persiste:

**Diagnóstico rápido:**

```powershell
# Verificar que el servidor responde (deberías ver JSON con "ok":true)
curl http://127.0.0.1:9101/health
curl http://[::1]:9101/health
```

**Si ninguna URL responde**, el servidor no se inició. Revisa logs en `%USERPROFILE%\.cegel\`.

**Si curl responde pero la app no**, ejecutar como Administrador:

```powershell
# Permitir loopback en WebView2 (causa más común en Windows 10/11)
CheckNetIsolation LoopbackExempt -a -n="Microsoft.Win32WebViewHost_cw5n1h2txyewy"

# Agregar regla de firewall
New-NetFirewallRule -DisplayName "Cegel Print Bridge" `
  -Direction Inbound -Protocol TCP -LocalPort 9101 `
  -Action Allow -Profile Private,Public
```

---

## Roadmap

- [x] Entrega 1–3: server local, adapters, scaffold Tauri
- [x] Entrega 4: pairing + heartbeat + autostart + multi-tenant
- [ ] Entrega 5: auto-update vía Tauri Updater (GitHub Releases)
- [ ] Adapter Bluetooth (BLE — `btleplug`)
- [ ] Cola persistente con reintentos para impresiones fallidas
