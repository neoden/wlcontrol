subdir('resources')

desktop_file = i18n.merge_file(
  input: 'dev.neoden.wlcontrol.desktop.in',
  output: 'dev.neoden.wlcontrol.desktop',
  type: 'desktop',
  po_dir: '../po',
  install: true,
  install_dir: get_option('datadir') / 'applications',
)

desktop_utils = find_program('desktop-file-validate', required: false)
if desktop_utils.found()
  test('Validate desktop file', desktop_utils, args: [desktop_file])
endif
