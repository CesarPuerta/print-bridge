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

1. Descargar el archivo `Cegel-Print-Bridge_x.y.z_universal.dmg`.
2. Doble clic → arrastrar el ícono **Cegel Print Bridge** a la carpeta **Aplicaciones**.
3. Primera ejecución:
   - Como el binario **no está firmado con certificado de Apple**, macOS lo bloquea por defecto.
   - **Solución (1 sola vez):** clic derecho sobre la app → **Abrir** → en el diálogo, **Abrir** otra vez.
   - Alternativa por terminal: `xattr -d com.apple.quarantine "/Applications/Cegel Print Bridge.app"`.
4. Conceder permisos cuando macOS los pida:
   - **USB** (al imprimir la primera vez): aceptar el diálogo del sistema.
   - **Accesibilidad / Inicio automático**: Sistema → Privacidad y Seguridad → Inicio de sesión → habilitar “Cegel Print Bridge”. La app intenta auto-registrarse en el primer arranque.
5. Verificar: el ícono aparece en la barra de menús (arriba a la derecha). Clic → debe decir “Estado: Activo en http://127.0.0.1:9101”.

### Windows

1. Descargar `Cegel-Print-Bridge_x.y.z_x64-setup.exe` (o el `.msi`).
2. Doble clic → ejecutar.
3. **Windows SmartScreen aparece** porque el binario no está firmado: clic en **Más información** → **Ejecutar de todos modos**.
4. Aceptar el UAC.
5. El instalador registra inicio automático con Windows (Task Scheduler).
6. Verificar: ícono en bandeja (esquina inferior derecha, junto al reloj). Clic derecho → “Estado: Activo”.

### Linux

1. Descargar `cegel-print-bridge_x.y.z_amd64.AppImage`.
2. Dar permiso de ejecución: `chmod +x cegel-print-bridge_*.AppImage`.
3. Doble clic o ejecutar desde terminal.
4. Para que el usuario pueda leer USB sin sudo (impresoras térmicas):
   ```bash
   sudo usermod -aG plugdev,dialout $USER
   sudo cp install/99-cegel-printers.rules /etc/udev/rules.d/
   sudo udevadm control --reload-rules
   ```
   (cerrar sesión y volver a entrar).
5. Para autostart, crear `~/.config/autostart/cegel-bridge.desktop` apuntando al AppImage.

---

## Configuración inicial / vinculación

1. Abrir la web app en el mismo equipo: `https://www.cegel.app`.
2. Ir a **Configuración → Equipos → Vincular nuevo**.
3. En el bridge, clic en el ícono de la bandeja → **Vincular equipo**.
4. El bridge mostrará un **código de 6 dígitos** durante 10 minutos.
5. Escribir ese código en la web → confirmar.
6. Listo: el bridge queda vinculado al negocio activo y comienza a enviar heartbeats cada 60 s.

El estado de vínculo se persiste en `~/.cegel/bridge.json`:

```json
{
  "port": 9101,
  "allowed_origins": ["https://www.cegel.app", "http://localhost:5173"],
  "paired_business_id": "65f...",
  "device_token": "<hex 64>",
  "device_id": "<uuid>",
  "cegel_api_base": "https://api.cegel.app"
}
```

Para desvincular: ventana del bridge → **Desvincular este equipo**.

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

## Roadmap

- [x] Entrega 1–3: server local, adapters, scaffold Tauri
- [x] Entrega 4: pairing + heartbeat + autostart + multi-tenant
- [ ] Entrega 5: auto-update vía Tauri Updater (GitHub Releases)
- [ ] Adapter Bluetooth (BLE — `btleplug`)
- [ ] Cola persistente con reintentos para impresiones fallidas
