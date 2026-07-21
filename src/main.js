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

async function pollPairing() {
  try {
    const state = await invoke('cmd_get_pairing_state');
    const codeBox = $('pairing-code-box');
    const errBox = $('pairing-error');
    errBox.textContent = '';

    switch (state.status) {
      case 'waiting':
        codeBox.style.display = '';
        $('pairing-code').textContent = state.pairing_code || '—';
        $('pairing-help').textContent =
          'Abre www.cegel.app → Equipos → Vincular nuevo, y escribe este código.';
        break;
      case 'paired':
        codeBox.style.display = 'none';
        stopPolling();
        await refresh();
        showMsg('✅ Equipo vinculado correctamente.');
        break;
      case 'error':
        codeBox.style.display = 'none';
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
    await refresh();
    showMsg('Equipo desvinculado.');
  } catch (err) {
    showMsg('Error al desvincular: ' + (err?.message || err));
  }
});

$('open-config').addEventListener('click', () => {
  showMsg('Tu config está en ~/.cegel/bridge.json');
});

refresh();
setInterval(refresh, 5000);

if (window.location.hash === '#/pair') {
  invoke('cmd_start_pairing')
    .then(startPolling)
    .catch(err => showMsg('Error: ' + err));
}
