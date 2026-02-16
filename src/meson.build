cargo_bin = find_program('cargo')
cargo_opt = ['--manifest-path', meson.project_source_root() / 'Cargo.toml']
cargo_opt += ['--target-dir', meson.project_build_root() / 'target']
cargo_env = ['CARGO_HOME=' + meson.project_build_root() / 'cargo-home']

if get_option('buildtype') == 'release'
  cargo_opt += ['--release']
  rust_target = 'release'
else
  rust_target = 'debug'
endif

cargo_build = custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: 'wlcontrol',
  console: true,
  install: true,
  install_dir: get_option('bindir'),
  depends: resources,
  command: [
    'env',
    cargo_env,
    cargo_bin, 'build',
    cargo_opt,
    '&&',
    'cp', meson.project_build_root() / 'target' / rust_target / 'wlcontrol', '@OUTPUT@',
  ],
)
