// 中文基准词表（模块化拆分；由 ../../zh-CN.ts 合并导出）。
export const system = {
  // ---- 管理员 ----
  'admin.super': '超级管理员',
  'admin.create_title': '新增管理员',
  'admin.edit_title': '编辑管理员：{username}',
  'admin.reset_pwd': '重置密码',
  'admin.reset_pwd_title': '重置 {username} 的密码',
  'admin.reset_pwd_placeholder': '新密码（至少 8 位，且需包含字母、数字、符号中的至少两类）',
  'admin.reset_done': '已重置',
  'admin.delete_confirm': '确认删除该管理员？',
  'admin.grant_hint': '灰掉的角色超出你的数据权限，无法授予',
  'admin.pick_dept': '选择部门',
  'admin.pick_roles': '选择角色',
  'admin.roles_truncated_hint': '仅显示前 100 条，请在上方输入搜索',

  // ---- 角色 ----
  'role.create_title': '新增角色',
  'role.edit_title': '编辑角色：{name}',
  'role.delete_confirm': '确认删除该角色？',
  'role.perm_title': '分配权限：{name}',
  'role.perm_saved': '权限已保存',
  'role.perm_button': '权限',
  'role.data_perm': '数据权限',
  'role.button_tag': '（按钮）',
  'role.code_example': '如 admin',

  // ---- 数据范围 ----
  'scope.all': '全部数据',
  'scope.dept_below': '本部门及以下',
  'scope.dept': '本部门',
  'scope.self': '仅本人',
  'scope.custom': '自定义',

  // ---- 菜单 ----
  'menu.type_dir': '目录',
  'menu.type_menu': '菜单',
  'menu.type_button': '按钮',
  'menu.show': '显示',
  'menu.hide': '隐藏',
  'menu.add_child': '新增子项',
  'menu.delete_confirm': '确认删除该菜单？',
  'menu.create_title': '新增菜单',
  'menu.edit_title': '编辑菜单：{name}',
  'menu.path_example': '如 /system/admin',
  'menu.component_hint': '前端组件路径，可留空',
  'menu.perm_example': '如 system:admin:add',
  'menu.icon_example': '如 SettingOutlined',

  // ---- 部门 ----
  'dept.add_child': '新增子部门',
  'dept.delete_confirm': '确认删除该部门？',
  'dept.create_title': '新增部门',
  'dept.edit_title': '编辑部门：{name}',

  // ---- 字典 ----
  'dict.types_title': '字典类型',
  'dict.items_title': '字典项',
  'dict.items_title_selected': '字典项：{name}（{code}）',
  'dict.type_create_title': '新增字典类型',
  'dict.type_edit_title': '编辑字典类型：{name}',
  'dict.item_create_title': '新增字典项',
  'dict.item_edit_title': '编辑字典项：{label}',
  'dict.type_delete_confirm': '确认删除字典类型「{name}」？',
  'dict.type_delete_desc': '该类型下的所有字典项会一并删除。',
  'dict.item_delete_confirm': '确认删除该字典项？',
  'dict.empty_hint': '请在左侧选择一个字典类型；还没有类型的话，点左栏的「新增」建一个。',
  'dict.code_hint_locked': '编码是字典项的关联键，创建后不可修改',
  'dict.code_example': '如 user_sex',
  'dict.name_example': '如 用户性别',
  'dict.label_placeholder': '下拉里显示的文字',
  'dict.value_placeholder': '存库的值，同类型内不可重复',
} as const;
