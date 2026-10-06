import { describe, expect, it } from 'vitest';
import { envelopeError } from './client';

describe('envelopeError', () => {
  it('code != 0 判为业务失败，带出后端消息', () => {
    expect(envelopeError({ code: 40301, msg: '没有权限', data: null })).toBe('没有权限');
  });

  it('code != 0 且 msg 为空时给默认文案', () => {
    expect(envelopeError({ code: 500, msg: '' })).toBe('请求失败');
  });

  it('code == 0 放行', () => {
    expect(envelopeError({ code: 0, msg: 'ok', data: { id: 1 } })).toBeNull();
  });

  it('非信封响应（blob / null / 没有数字 code）放行，不能误判成失败', () => {
    expect(envelopeError(new Blob(['id,name\n1,a\n']))).toBeNull();
    expect(envelopeError(null)).toBeNull();
    expect(envelopeError(undefined)).toBeNull();
    expect(envelopeError('plain text')).toBeNull();
    expect(envelopeError({ code: '0', msg: 'ok' })).toBeNull();
  });

  it('有 err 且词表命中：用词表文案（带 args 占位）', () => {
    expect(envelopeError({ code: 400, msg: '用户名或密码错误', err: 'auth.bad_credentials' }))
      .toBe('用户名或密码错误');
    expect(envelopeError({
      code: 400, msg: '备注长度不能超过 255 个字符',
      err: 'common.too_long', args: { field: 'remark', max: 255 },
    })).toBe('备注 长度不能超过 255 个字符');
  });

  it('err 不在词表里：回落 msg，不能吞掉后端的话', () => {
    expect(envelopeError({ code: 400, msg: '这个码前端还没见过', err: 'brand.new.code' }))
      .toBe('这个码前端还没见过');
    expect(envelopeError({ code: 500, msg: '', err: 'brand.new.code' })).toBe('请求失败');
  });
});
