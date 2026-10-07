import { useCallback, useEffect, useState } from 'react';
import { Badge, Button, Empty, Popover, Spin, Typography } from 'antd';
import { BellOutlined } from '@ant-design/icons';
import { noticeApi } from '../api/notice';
import { applyRead } from '../hooks/noticeUnread';
import { useI18n } from '../i18n';
import type { Notice, NoticeUnread } from '../api/types';

/**
 * 顶栏未读公告铃铛。
 *
 * 只在挂载时拉一次 + 动作后刷新，**不做轮询**（v1.6.0 明确不做）。
 * 角标用接口的 total 而不是 list.length —— list 只给最近 50 条，超了会少报。
 */
export default function NoticeBell() {
  const { t } = useI18n();
  const [data, setData] = useState<NoticeUnread>({ total: 0, list: [] });
  const [loading, setLoading] = useState(true);
  const [marking, setMarking] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setData(await noticeApi.unread());
    } catch {
      // 拦截器已统一提示；拉不到就保持原状，不能让它把整个顶栏带崩
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const markRead = async (n: Notice) => {
    setMarking(n.id);
    try {
      await noticeApi.read(n.id);
      // 成功后才本地移除（角标减一）：失败时条目留在列表里，用户能重试
      setData((d) => applyRead(d, n.id));
    } catch {
      // 拦截器已统一提示
    } finally {
      setMarking(null);
    }
  };

  const rest = data.total - data.list.length;

  const content = (
    <div style={{ width: 340, maxHeight: 400, overflow: 'auto' }}>
      {loading && data.list.length === 0 ? (
        <div style={{ textAlign: 'center', padding: 16 }}><Spin /></div>
      ) : data.list.length === 0 ? (
        <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t('notice.unread_empty')} />
      ) : (
        data.list.map((n) => (
          <div key={n.id} style={{ borderBottom: '1px solid rgba(5,5,5,0.06)', padding: '8px 0' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', gap: 8 }}>
              <Typography.Text strong>{n.title}</Typography.Text>
              <Typography.Text type="secondary" style={{ fontSize: 12, whiteSpace: 'nowrap' }}>
                {n.published_at ?? ''}
              </Typography.Text>
            </div>
            <Typography.Paragraph type="secondary" style={{ margin: '4px 0', whiteSpace: 'pre-wrap' }}>
              {n.content}
            </Typography.Paragraph>
            <Button
              size="small"
              type="link"
              style={{ padding: 0 }}
              loading={marking === n.id}
              onClick={() => void markRead(n)}
            >
              {t('notice.mark_read')}
            </Button>
          </div>
        ))
      )}
      {rest > 0 && (
        <Typography.Text type="secondary" style={{ fontSize: 12 }}>
          {t('notice.unread_rest', { n: rest })}
        </Typography.Text>
      )}
    </div>
  );

  return (
    <Popover content={content} title={t('notice.unread_title', { n: data.total })} trigger="click" placement="bottomRight">
      <Badge count={data.total} size="small" overflowCount={99} offset={[-2, 2]}>
        <BellOutlined style={{ fontSize: 18, cursor: 'pointer' }} />
      </Badge>
    </Popover>
  );
}
