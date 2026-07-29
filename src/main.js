const $ = id => document.getElementById(id);
const invoke = window.__TAURI__?.core?.invoke;

// Mostrar errores en la UI en lugar de usar alert()
function showMsg(msg) {
  const box = $('pairing-error');
  if (box) {
    box.textContent = msg;
    box.style.display = '';
    setTimeout(() => {
      box.style.display = 'none';
    }, 8000);
  }
}

if (!invoke) {
  document.body.innerHTML = '<p style="color:red;padding:20px">Error: Tauri API no disponible</p>';
  throw new Error('Tauri API no disponible');
}

async function refresh() {
  try {
    const status = await invoke('cmd_get_status');
    $('status-dot').className = 'dot ' + (status.ok ? 'dot--on' : 'dot--off');
    $('status-text').textContent = status.ok ? 'Activo' : 'Detenido';
    $('port').textContent = status.port;
    $('version').textContent = status.version;
    $('paired').textContent = status.paired ? 'Sí' : 'No';
    $('device-id').textContent = status.device_id;
    $('business-id').textContent = status.business_id || '—';

    $('unpair').style.display = status.paired ? '' : 'none';
    $('pair').style.display = status.paired ? 'none' : '';

    // Health check desde Rust (evita restricciones de red de WebView2 en Windows)
    let serverOk = false;
    try {
      serverOk = await invoke('cmd_check_health');
    } catch {
      // Si el comando falla, asumimos servidor no disponible
    }

    if (!serverOk) {
      $('status-dot').className = 'dot dot--off';
      $('status-text').textContent = `Servidor no responde en puerto ${status.port}`;
    }
  } catch (err) {
    showMsg('Error al obtener estado: ' + (err?.message || err));
  }
}

let pollHandle = null;

let lastPairingCode = null;

async function pollPairing() {
  try {
    const state = await invoke('cmd_get_pairing_state');
    const codeBox = $('pairing-code-box');
    const errBox = $('pairing-error');
    errBox.textContent = '';

    switch (state.status) {
      case 'waiting': {
        codeBox.style.display = '';
        // Solo actualizar si el código cambió, para no romper la selección de texto
        const code = state.pairing_code || '—';
        if (code !== lastPairingCode) {
          lastPairingCode = code;
          $('pairing-code').textContent = code;
        }
        $('pairing-help').textContent =
          'Abre www.cegel.app → Equipos → Vincular nuevo, y escribe este código.';
        break;
      }
      case 'paired':
        codeBox.style.display = 'none';
        lastPairingCode = null;
        stopPolling();
        await refresh();
        showMsg('✅ Equipo vinculado correctamente.');
        break;
      case 'error':
        codeBox.style.display = 'none';
        lastPairingCode = null;
        errBox.textContent = state.error || 'Error en la vinculación.';
        stopPolling();
        break;
      default:
        codeBox.style.display = 'none';
    }
  } catch (err) {
    showMsg('Error en vinculación: ' + (err?.message || err));
  }
}

function startPolling() {
  if (pollHandle) return;
  pollHandle = setInterval(pollPairing, 2000);
  pollPairing();
}

function stopPolling() {
  if (pollHandle) {
    clearInterval(pollHandle);
    pollHandle = null;
  }
}

$('pair').addEventListener('click', async () => {
  // Limpiar info anterior para facilitar re-vinculación
  $('device-id').textContent = '—';
  $('business-id').textContent = '—';
  $('paired').textContent = 'No';
  stopPolling();
  $('pairing-code-box').style.display = 'none';

  showMsg('Iniciando vinculación…');
  try {
    await invoke('cmd_start_pairing');
    startPolling();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    showMsg('No se pudo iniciar la vinculación: ' + message);
  }
});

let unpairPending = false;
$('unpair').addEventListener('click', async () => {
  if (!unpairPending) {
    unpairPending = true;
    $('unpair').textContent = '¿Confirmar desvincular?';
    $('unpair').className = 'danger';
    showMsg('Haz clic de nuevo para confirmar la desvinculación.');
    setTimeout(() => {
      unpairPending = false;
      $('unpair').textContent = 'Desvincular';
      $('unpair').className = 'danger';
    }, 5000);
    return;
  }
  unpairPending = false;
  $('unpair').textContent = 'Desvincular';
  try {
    await invoke('cmd_unpair');
    // Limpiar toda la info del equipo
    $('device-id').textContent = '—';
    $('business-id').textContent = '—';
    $('paired').textContent = 'No';
    $('unpair').style.display = 'none';
    $('pair').style.display = '';
    showMsg('Equipo desvinculado.');
  } catch (err) {
    showMsg('Error al desvincular: ' + (err?.message || err));
  }
});

// ── Printer management ─────────────────────────────────────────────────────

const BRIDGE_URL = 'http://localhost:9101';

async function scanUsb() {
  const btn = $('scan-usb');
  const list = $('usb-list');
  btn.disabled = true;
  btn.textContent = '⏳ Detectando…';
  list.innerHTML = '';

  try {
    const res = await fetch(`${BRIDGE_URL}/usb-devices`);
    const data = await res.json();
    const devices = data.devices || [];

    if (devices.length === 0) {
      list.innerHTML = '<p class="muted small">No se encontraron impresoras USB.</p>';
      return;
    }

    list.innerHTML = devices
      .map(
        d => `
      <div class="printer-item">
        <div>
          <strong>${d.product || d.manufacturer || 'Impresora térmica'}</strong>
          <span class="muted small">USB ${d.vendorId}:${d.productId}</span>
        </div>
        <div class="printer-item__actions">
          <button class="test-usb small" data-vid="${d.vendorId}" data-pid="${d.productId}">
            Probar
          </button>
          <button class="config-printer" data-vid="${d.vendorId}" data-pid="${d.productId}">
            Configurar
          </button>
        </div>
      </div>
    `,
      )
      .join('');

    list.querySelectorAll('.test-usb').forEach(btn => {
      btn.addEventListener('click', () => testUsbDevice(btn.dataset.vid, btn.dataset.pid));
    });
    list.querySelectorAll('.config-printer').forEach(btn => {
      btn.addEventListener('click', () => configurePrinter(btn.dataset.vid, btn.dataset.pid));
    });
  } catch (err) {
    // Mostrar más info para diagnosticar en Windows
    const detail = err.message || 'Error de conexión';
    list.innerHTML = `<p class="error">Error al escanear: ${detail}</p>
      <p class="muted small">¿Está corriendo el servidor? Verificá el estado arriba (debe decir "Activo").</p>`;
  } finally {
    btn.disabled = false;
    btn.textContent = '🔍 Detectar impresora USB';
  }
}

async function testUsbDevice(vendorId, productId) {
  $('printer-msg').textContent = '⏳ Enviando prueba...';
  try {
    const res = await fetch(`${BRIDGE_URL}/usb/test`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ vendorId, productId }),
    });
    const data = await res.json();
    if (data.ok) {
      $('printer-msg').textContent =
        '✅ Ticket de prueba enviado. Si la impresora imprime, configurala.';
    } else {
      $('printer-msg').textContent = `❌ ${data.error || 'Error al probar'}`;
    }
  } catch (err) {
    $('printer-msg').textContent = `❌ ${err.message}`;
  }
}

let configuringDevice = null;

async function editPrinter(id, currentName, drawerPin, paperWidth) {
  if (configuringDevice) {
    document.querySelector('.config-form-inline')?.remove();
  }
  configuringDevice = { vendorId: null, productId: null, editId: id };

  const list = $('printer-list');
  const form = document.createElement('div');
  form.className = 'config-form-inline';
  form.innerHTML = `
    <input type="text" id="printer-name-input" class="config-name-input"
           placeholder="Nombre" value="${currentName}" autofocus />
    <select id="printer-pin-input" class="config-pin-select">
      <option value="0" ${drawerPin == '0' ? 'selected' : ''}>Sin cajón</option>
      <option value="2" ${drawerPin == '2' ? 'selected' : ''}>Cajón pin 2</option>
      <option value="5" ${drawerPin == '5' ? 'selected' : ''}>Cajón pin 5</option>
    </select>
    <button id="confirm-edit" class="primary small">Guardar</button>
    <button id="cancel-edit" class="small">Cancelar</button>
  `;
  list.appendChild(form);

  const input = form.querySelector('#printer-name-input');
  input.focus();
  input.select();

  const cleanup = () => {
    form.remove();
    configuringDevice = null;
  };

  form.querySelector('#cancel-edit').addEventListener('click', cleanup);

  form.querySelector('#confirm-edit').addEventListener('click', async () => {
    const name = input.value.trim();
    if (!name) return;
    const pin = parseInt(form.querySelector('#printer-pin-input').value);

    cleanup();
    $('printer-msg').textContent = '⏳ Guardando...';

    try {
      const res = await fetch(`${BRIDGE_URL}/printers/configure`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id, name, paperWidthMm: parseInt(paperWidth), drawerPin: pin }),
      });
      if (!res.ok) {
        const err = await res.json();
        throw new Error(err.error || 'Error al guardar');
      }
      $('printer-msg').textContent = `✅ "${name}" actualizada.`;
      loadPrinters();
    } catch (err) {
      $('printer-msg').textContent = `❌ ${err.message}`;
    }
  });

  input.addEventListener('keydown', e => {
    if (e.key === 'Enter') form.querySelector('#confirm-edit').click();
    if (e.key === 'Escape') cleanup();
  });
}

async function configurePrinter(vendorId, productId) {
  // Si ya estamos configurando otro, cancelar
  if (configuringDevice) {
    document.querySelector('.config-form-inline')?.remove();
  }
  configuringDevice = { vendorId, productId };

  const list = $('usb-list');
  const form = document.createElement('div');
  form.className = 'config-form-inline';
  form.innerHTML = `
    <input type="text" id="printer-name-input" class="config-name-input" 
           placeholder="Nombre de la impresora" value="POS ${vendorId}" autofocus />
    <select id="printer-pin-input" class="config-pin-select">
      <option value="0">Sin cajón</option>
      <option value="2">Cajón pin 2</option>
      <option value="5">Cajón pin 5</option>
    </select>
    <button id="confirm-config" class="primary small">Guardar</button>
    <button id="cancel-config" class="small">Cancelar</button>
  `;
  list.appendChild(form);

  const input = form.querySelector('#printer-name-input');
  input.focus();
  input.select();

  const cleanup = () => {
    form.remove();
    configuringDevice = null;
  };

  form.querySelector('#cancel-config').addEventListener('click', cleanup);

  form.querySelector('#confirm-config').addEventListener('click', async () => {
    const name = input.value.trim();
    if (!name) return;
    const pin = parseInt(form.querySelector('#printer-pin-input').value);

    cleanup();
    $('printer-msg').textContent = '⏳ Configurando...';

    try {
      const res = await fetch(`${BRIDGE_URL}/printers/configure`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name,
          paperWidthMm: 80,
          drawerPin: pin,
          connection: {
            type: 'usb',
            vendorId,
            productId,
          },
        }),
      });

      if (!res.ok) {
        const err = await res.json();
        throw new Error(err.error || 'Error al configurar');
      }

      $('printer-msg').textContent = `✅ "${name}" configurada correctamente.`;
      loadPrinters();
    } catch (err) {
      $('printer-msg').textContent = `❌ ${err.message}`;
    }
  });

  input.addEventListener('keydown', e => {
    if (e.key === 'Enter') form.querySelector('#confirm-config').click();
    if (e.key === 'Escape') cleanup();
  });
}

async function loadPrinters() {
  const list = $('printer-list');
  try {
    const res = await fetch(`${BRIDGE_URL}/printers`);
    const data = await res.json();
    const printers = data.printers || [];

    if (printers.length === 0) {
      list.innerHTML = '';
      return;
    }

    list.innerHTML = printers
      .map(
        p => `
      <div class="printer-item ${p.online ? 'printer-item--online' : ''}">
        <div>
          <strong>${p.name}</strong>
          <span class="muted small">${p.paperWidthMm}mm${p.drawerPin > 0 ? ` · Cajón pin ${p.drawerPin}` : ''} · ${p.online ? '🟢 Conectada' : '⚪ Sin verificar'}</span>
        </div>
        <div class="printer-item__actions">
          <button class="test-printer small" data-id="${p.id}">Probar</button>
          <button class="edit-printer small" data-id="${p.id}" data-name="${p.name}" data-pin="${p.drawerPin || 0}" data-width="${p.paperWidthMm}">Editar</button>
          <button class="delete-printer small danger" data-id="${p.id}">Eliminar</button>
        </div>
      </div>
    `,
      )
      .join('');

    list.querySelectorAll('.test-printer').forEach(btn => {
      btn.addEventListener('click', () => testPrinter(btn.dataset.id));
    });
    list.querySelectorAll('.edit-printer').forEach(btn => {
      btn.addEventListener('click', () =>
        editPrinter(btn.dataset.id, btn.dataset.name, btn.dataset.pin, btn.dataset.width),
      );
    });
    list.querySelectorAll('.delete-printer').forEach(btn => {
      btn.addEventListener('click', () => deletePrinter(btn.dataset.id));
    });
  } catch {
    // bridge not running
  }
}

async function testPrinter(id) {
  try {
    const res = await fetch(`${BRIDGE_URL}/printers/test`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id }),
    });
    const data = await res.json();
    if (data.ok) {
      $('printer-msg').textContent = '✅ Ticket de prueba enviado.';
    } else {
      $('printer-msg').textContent = `❌ ${data.error}`;
    }
  } catch (err) {
    $('printer-msg').textContent = `❌ ${err.message}`;
  }
}

async function deletePrinter(id) {
  // confirm() no funciona en WebView de Tauri — usar doble-click
  const btn = document.querySelector(`.delete-printer[data-id="${id}"]`);
  if (!btn || btn.dataset.deleting !== 'true') {
    if (btn) {
      btn.dataset.deleting = 'true';
      btn.textContent = '¿Eliminar?';
      btn.classList.add('danger');
      setTimeout(() => {
        btn.dataset.deleting = 'false';
        btn.textContent = 'Eliminar';
        btn.classList.remove('danger');
      }, 4000);
    }
    return;
  }
  try {
    const res = await fetch(`${BRIDGE_URL}/printers/delete`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id }),
    });
    if (!res.ok) {
      const data = await res.json().catch(() => ({}));
      throw new Error(data.error || `Error ${res.status}`);
    }
    $('printer-msg').textContent = 'Impresora eliminada.';
    await loadPrinters();
  } catch (err) {
    $('printer-msg').textContent = `❌ ${err.message}`;
  }
}

$('scan-usb').addEventListener('click', scanUsb);
loadPrinters();

refresh();
setInterval(refresh, 5000);

if (window.location.hash === '#/pair') {
  invoke('cmd_start_pairing')
    .then(startPolling)
    .catch(err => showMsg('Error: ' + err));
}
