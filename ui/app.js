(() => {
  const $ = (id) => document.getElementById(id);
  const els = {
    process: $('process'),
    dllName: $('dll-name'),
    inject: $('inject'),
    auto: $('auto'),
    procStatus: $('proc-status'),
  };
  const send = (m) => window.ipc && window.ipc.postMessage(JSON.stringify(m));

  window.__set_state = (s) => {
    if (document.activeElement !== els.process && els.process.value !== s.process) {
      els.process.value = s.process;
    }
    els.process.placeholder = s.processDefault;

    if (s.dllName) {
      els.dllName.textContent = s.dllName;
      els.dllName.className = 'dll-name';
      els.dllName.title = s.dllPath || s.dllName;
    } else if (s.dllError) {
      els.dllName.textContent = s.dllError;
      els.dllName.className = 'dll-name error';
      els.dllName.title = '';
    } else {
      els.dllName.textContent = 'no DLL selected';
      els.dllName.className = 'dll-name muted';
      els.dllName.title = '';
    }

    const busy = s.pending.length > 0;
    els.inject.disabled = !s.dllName || busy;
    els.inject.textContent = busy ? 'Injecting…' : 'Inject';
    els.inject.setAttribute('aria-busy', busy ? 'true' : 'false');

    els.auto.checked = s.auto;

    if (s.pids.length === 0) {
      els.procStatus.textContent = `${s.process}: not found`;
      els.procStatus.className = 'status-line proc-off';
    } else {
      els.procStatus.textContent = `${s.process}: PID ${s.pids.join(', ')}`;
      els.procStatus.className = 'status-line proc-on';
    }
  };

  document.getElementById('browse').addEventListener('click', () => send({ cmd: 'pick_dll' }));
  els.inject.addEventListener('click', () => send({ cmd: 'inject' }));
  els.auto.addEventListener('change', () => send({ cmd: 'set_auto', value: els.auto.checked }));

  let procTimer = 0;
  els.process.addEventListener('input', () => {
    clearTimeout(procTimer);
    procTimer = setTimeout(() => send({ cmd: 'set_process', value: els.process.value }), 300);
  });

  send({ cmd: 'hello' });
})();
