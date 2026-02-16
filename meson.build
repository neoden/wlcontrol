project('wlcontrol',
  'rust',
  version: '0.1.0',
  meson_version: '>= 1.0.0',
  default_options: ['warning_level=2', 'werror=false'],
)

i18n = import('i18n')
gnome = import('gnome')

base_id = 'dev.neoden.wlcontrol'

dependency('gtk4', version: '>= 4.16')
dependency('libadwaita-1', version: '>= 1.6')

find_program('cargo')
find_program('blueprint-compiler')

subdir('data')
subdir('src')

gnome.post_install(
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)
