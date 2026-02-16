blueprints = custom_target('blueprints',
  input: files(
    'ui/window.blp',
    'ui/wifi-page.blp',
    'ui/bluetooth-page.blp',
    'ui/wifi-network-row.blp',
    'ui/bluetooth-device-row.blp',
    'ui/password-dialog.blp',
  ),
  output: '.',
  command: [
    find_program('blueprint-compiler'),
    'batch-compile',
    '@OUTPUT@',
    '@CURRENT_SOURCE_DIR@',
    '@INPUT@',
  ],
)

resources = gnome.compile_resources(
  'wlcontrol',
  'resources.gresource.xml',
  gresource_bundle: true,
  dependencies: blueprints,
  source_dir: meson.current_build_dir(),
  install: true,
  install_dir: get_option('datadir') / 'wlcontrol',
)
