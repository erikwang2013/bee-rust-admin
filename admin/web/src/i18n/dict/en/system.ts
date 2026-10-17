// English dictionary (module split; merged by ../../en-US.ts).
export const system = {
  // ---- Admins ----
  'admin.super': 'Super admin',
  'admin.create_title': 'New admin',
  'admin.edit_title': 'Edit admin: {username}',
  'admin.reset_pwd': 'Reset password',
  'admin.reset_pwd_title': 'Reset the password of {username}',
  'admin.reset_pwd_placeholder': 'New password (at least 8 characters, with at least two of letters/digits/symbols)',
  'admin.reset_done': 'Password reset',
  'admin.delete_confirm': 'Delete this admin?',
  'admin.grant_hint': 'Greyed-out roles exceed your data scope and cannot be granted',
  'admin.pick_dept': 'Select department',
  'admin.pick_roles': 'Select roles',
  'admin.roles_truncated_hint': 'Only the first 100 roles are shown — type above to search',

  // ---- Roles ----
  'role.create_title': 'New role',
  'role.edit_title': 'Edit role: {name}',
  'role.delete_confirm': 'Delete this role?',
  'role.perm_title': 'Assign permissions: {name}',
  'role.perm_saved': 'Permissions saved',
  'role.perm_button': 'Permissions',
  'role.data_perm': 'Data permissions',
  'role.button_tag': ' (button)',
  'role.code_example': 'e.g. admin',

  // ---- Data scope ----
  'scope.all': 'All data',
  'scope.dept_below': 'Department and below',
  'scope.dept': 'Department',
  'scope.self': 'Self only',
  'scope.custom': 'Custom',

  // ---- Menus ----
  'menu.type_dir': 'Directory',
  'menu.type_menu': 'Menu',
  'menu.type_button': 'Button',
  'menu.show': 'Visible',
  'menu.hide': 'Hidden',
  'menu.add_child': 'Add child',
  'menu.delete_confirm': 'Delete this menu?',
  'menu.create_title': 'New menu',
  'menu.edit_title': 'Edit menu: {name}',
  'menu.path_example': 'e.g. /system/admin',
  'menu.component_hint': 'Frontend component path, optional',
  'menu.perm_example': 'e.g. system:admin:add',
  'menu.icon_example': 'e.g. SettingOutlined',

  // ---- Departments ----
  'dept.add_child': 'Add sub-department',
  'dept.delete_confirm': 'Delete this department?',
  'dept.create_title': 'New department',
  'dept.edit_title': 'Edit department: {name}',

  // ---- Dictionaries ----
  'dict.types_title': 'Dictionary types',
  'dict.items_title': 'Dictionary items',
  'dict.items_title_selected': 'Dictionary items: {name} ({code})',
  'dict.type_create_title': 'New dictionary type',
  'dict.type_edit_title': 'Edit dictionary type: {name}',
  'dict.item_create_title': 'New dictionary item',
  'dict.item_edit_title': 'Edit dictionary item: {label}',
  'dict.type_delete_confirm': 'Delete dictionary type "{name}"?',
  'dict.type_delete_desc': 'All dictionary items under this type will be deleted as well.',
  'dict.item_delete_confirm': 'Delete this dictionary item?',
  'dict.empty_hint': 'Pick a dictionary type on the left; if there is none yet, click "New" in the left column to create one.',
  'dict.code_hint_locked': 'The code links items to this type and cannot be changed after creation',
  'dict.code_example': 'e.g. user_sex',
  'dict.name_example': 'e.g. User gender',
  'dict.label_placeholder': 'Text shown in dropdowns',
  'dict.value_placeholder': 'Value stored in the database, unique within the type',
} as const;
