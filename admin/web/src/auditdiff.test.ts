import { describe, expect, it } from 'vitest';
import { changesText } from './auditdiff';
import { t } from './i18n';

describe('changesText：审计变更摘要 → 一行文本', () => {
  it('普通字段：标签: 旧 → 新', () => {
    const raw = '{"nickname":{"old":"旧昵称","new":"新昵称"}}';
    expect(changesText(raw, t)).toBe('昵称: 旧昵称 → 新昵称');
  });

  it('密文列（changed 标记）：只显示「已变更」，值不出现', () => {
    expect(changesText('{"email":{"changed":true}}', t)).toBe('邮箱: 已变更');
  });

  it('多字段按出现顺序用分号连接；未知字段名原样显示', () => {
    const raw = '{"name":{"old":"a","new":"b"},"weird":{"old":1,"new":2}}';
    expect(changesText(raw, t)).toBe('名称: a → b; weird: 1 → 2');
  });

  it('空串与坏 JSON：空串回空、坏 JSON 原样回显（坏数据也要看得见）', () => {
    expect(changesText('', t)).toBe('');
    expect(changesText(undefined, t)).toBe('');
    expect(changesText('not json', t)).toBe('not json');
    expect(changesText('[1,2]', t)).toBe('[1,2]');
  });
});
