-- 0011 — Seed the permission registry.
-- Must stay in step with casual-task-model::permission::ALL. The
-- permission-parity test asserts the two agree (docs/15).

INSERT INTO permission (key, description, added_in) VALUES
  ('workspace.manage',        'Manage workspace settings',                    'v1'),
  ('workspace.delete',        'Delete the workspace',                         'v1'),
  ('workspace.owner',         'Workspace ownership; the last one is protected','v1'),
  ('project.create',          'Create projects',                              'v1'),
  ('project.update',          'Update project settings',                      'v1'),
  ('project.delete',          'Delete a project',                             'v1'),
  ('project.member.manage',   'Add and remove project members',               'v1'),
  ('project.role.assign',     'Assign roles within a project',                'v1'),
  ('project.workflow.manage', 'Configure statuses and transitions',           'v1'),
  ('task.read',               'Read tasks',                                   'v1'),
  ('task.create',             'Create tasks',                                 'v1'),
  ('task.update',             'Update task fields',                           'v1'),
  ('task.assign',             'Assign tasks',                                 'v1'),
  ('task.move',               'Move a task between projects',                 'v1'),
  ('task.transition',         'Transition a task between statuses',           'v1'),
  ('task.close',              'Transition a task into a COMPLETED status',    'v1'),
  ('task.reopen',             'Transition a task out of a terminal status',   'v1'),
  ('task.delete',             'Delete tasks',                                 'v1'),
  ('task.comment',            'Comment on tasks',                             'v1'),
  ('task.history.read',       'Read task activity',                           'v1'),
  ('task.dependency.override','Transition despite unresolved blockers',       'v1'),
  ('task.attachment.create',  'Upload attachments',                           'v1'),
  ('task.attachment.read',    'Download attachments',                         'v1'),
  ('tag.manage',              'Create and edit tags',                         'v1'),
  ('role.manage',             'Author roles (workspace scope only)',          'v1'),
  ('audit.read',              'Read the audit stream',                        'v1'),
  ('plugin.install',          'Install and configure plugins',                'v1'),
  ('automation.manage',       'Author automation rules',                      'v1')
ON CONFLICT (key) DO NOTHING;
