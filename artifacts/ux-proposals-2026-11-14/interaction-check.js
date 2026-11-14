
(() => {
  const root = document.getElementById('rill-ux-proposals');
  const choice = root.querySelector('#rill-proposal-choice');
  const meta = root.querySelector('#rill-proposal-meta');
  const descriptions = {
    P1: '5/5 sessions · First priority · Medium scope',
    P2: '3/5 sessions · First priority · Medium–large scope: original samples + loading',
    P3: '1/5 sessions + saved-project proof · First priority · Medium scope',
    P4: '4/5 noticed device counts · Next priority · Small–medium scope',
    P5: '1/5 sessions · Next priority · Small–medium scope',
    P6: '2/5 sessions · Next priority · Small scope'
  };
  function show(id) {
    if (!descriptions[id]) id = 'P1';
    choice.value = id;
    root.querySelectorAll('[data-proposal]').forEach(panel => { panel.hidden = panel.dataset.proposal !== id; });
    meta.textContent = descriptions[id];
  }
  show(window.openai?.widgetState?.privateContent?.proposal || 'P1');
  choice.addEventListener('change', () => {
    show(choice.value);
    window.openai?.setWidgetState?.({ modelContent: null, privateContent: { proposal: choice.value } })?.catch(() => {});
  });
  window.addEventListener('openai:set_globals', event => {
    const state = event.detail?.globals?.widgetState?.privateContent;
    if (state?.proposal) show(state.proposal);
  });
  root.querySelector('[data-typing]').addEventListener('click', () => {
    const keyboard = root.querySelector('.rill-typing'); keyboard.hidden = !keyboard.hidden;
  });
  root.querySelector('[data-before-edit]').addEventListener('dblclick', () => {
    root.querySelector('[data-before-dialog] input').focus();
  });
  root.querySelector('[data-after-edit]').addEventListener('dblclick', () => {
    root.querySelector('[data-after-editor] button').focus();
  });
  root.querySelectorAll('[data-demo-pads] button').forEach(button => button.addEventListener('click', () => {
    root.querySelectorAll('[data-demo-pads] button').forEach(pad => pad.classList.remove('is-hit'));
    button.classList.add('is-hit');
    root.querySelector('[data-pad-status]').textContent = button.dataset.pad + ' selected · Mockup only, no audio';
  }));
  root.querySelectorAll('.rill-steps button').forEach(button => button.addEventListener('click', () => {
    const active = button.classList.toggle('is-note'); button.setAttribute('aria-pressed', String(active));
    root.querySelector('[data-drum-status]').textContent = active ? 'Note added to the mapped pad row' : 'Note removed from the mapped pad row';
  }));
  root.querySelectorAll('.rill-row-label').forEach(button => button.addEventListener('click', () => {
    root.querySelector('[data-drum-status]').textContent = button.dataset.pad + ' selected · Mockup only, no audio';
  }));
  root.querySelectorAll('[data-other-notes]').forEach(button => button.addEventListener('click', () => {
    root.querySelector('[data-drum-status]').textContent = 'Piano roll: unmapped notes remain available and unchanged';
  }));
  root.querySelector('[data-add-sound]').addEventListener('click', () => {
    const select = root.querySelector('[data-proposal="P4"] .rill-device select');
    root.querySelector('[data-sound-status]').textContent = 'Preset · ' + select.value;
  });
  root.querySelector('[data-scale-choice]').addEventListener('change', event => {
    const outside = event.target.value === 'minor';
    root.querySelector('[data-third]').classList.toggle('rill-outside', outside);
    root.querySelector('[data-scale-status]').textContent = outside ? 'E4 · Outside the selected scale guide · Notes unchanged' : 'C major guide · C major chord';
  });
  root.querySelectorAll('[data-result]').forEach(button => button.addEventListener('click', () => {
    root.querySelector('[data-action-status]').textContent = button.dataset.result;
  }));
  root.querySelector('[data-export-menu]').addEventListener('click', () => { root.querySelector('[data-export-type]').focus(); });
  root.querySelector('[data-export-type]').addEventListener('change', event => {
    const type = event.target.value; const depth = root.querySelector('[data-bit-depth]');
    depth.hidden = type === 'MP3' || type === 'Ogg Vorbis';
    depth.querySelector('option[value="32-bit float"]').disabled = type === 'FLAC';
    if (type === 'FLAC') depth.querySelector('select').value = '24-bit PCM';
  });
})();
