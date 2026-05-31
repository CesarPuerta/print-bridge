import { invoke } from '@tauri-apps/api/core';

const $ = id => document.getElementById(id);

async function refresh() {
  const status = await invoke('cmd_get_status');
  $('status-dot').className = 'dot ' + (status.ok ? 'dot--on' : 'dot--off');
  $('status-text').textContent = status.ok
    ? `Activo en http://127.0.0.1:${status.port}`
    : 'Detenido';
  $('port').textContent = status.port;
  $('version').textContent = status.version;
  $('paired').textContent = status.paired ? 'Sí' : 'No';
  $('device-id').textContent = status.device_id;
  $('business-id').textContent = status.business_id || '—';

  $('unpair').style.display = status.paired ? '' : 'none';
  $('pair').style.display = status.paired ? 'none' : '';

  try {
    const res = await fetch(`http://127.0.0.1:${status.port}/health`);
    if (!res.ok) throw new Error();
  } catch {
    $('status-dot').className = 'dot dot--off';
    $('status-text').textContent = 'El servidor no responde';
  }
}

let pollHandle = null;

async function pollPairing() {
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
      alert('Equipo vinculado correctamente.');
      break;
    case 'error':
      codeBox.style.display = 'none';
      errBox.textContent = state.error || 'Error en la vinculación.';
      stopPolling();
      break;
    default:
      codeBox.style.display = 'none';
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
  await invoke('cmd_start_pairing');
  startPolling();
});

$('unpair').addEventListener('click', async () => {
  if (!confirm('¿Desvincular este equipo? Tendrás que volver a vincularlo para imprimir.')) return;
  await invoke('cmd_unpair');
  await refresh();
});

$('open-config').addEventListener('click', () => {
  alert('Tu config está en ~/.cegel/bridge.json');
});

$('check-update').addEventListener('click', async () => {
  const btn = $('check-update');
  btn.disabled = true;
  btn.textContent = 'Buscando…';
  try {
    const res = await invoke('cmd_check_update');
    if (res.available) {
      alert(`Actualización ${res.version} descargada. El bridge se reiniciará para aplicarla.`);
    } else {
      alert(`Estás en la última versión (${res.current}).`);
    }
  } catch (e) {
    alert(`Error buscando actualizaciones: ${e}`);
  } finally {
    btn.disabled = false;
    btn.textContent = 'Buscar actualizaciones';
  }
});

refresh();
setInterval(refresh, 5000);

if (window.location.hash === '#/pair') {
  invoke('cmd_start_pairing').then(startPolling);
}
