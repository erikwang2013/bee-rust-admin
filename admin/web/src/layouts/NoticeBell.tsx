import { useCallback, useEffect, useState } from 'react';
import { Badge, Button, Empty, Modal, Popover, Spin, Typography } from 'antd';
import BellOutlined from '@ant-design/icons/es/icons/BellOutlined';
import { noticeApi } from '../api/notice';
import { applyRead } from '../hooks/noticeUnread';
import { useI18n } from '../i18n';
import type { Notice, NoticeSummary, NoticeUnread } from '../api/types';

/**
 * 顶栏未读公告铃铛。
 *
 * 只在挂载时拉一次 + 动作后刷新，**不做轮询**（v1.6.0 明确不做）。
 * 角标用接口的 total 而不是 list.length —— list 只给最近 50 条，超了会少报。
 * 未读行只有摘要：点「查看全文」才取正文，拉到后打开弹窗并标记已读。
 */
export default function NoticeBell() {
  const { t } = useI18n();
  const [data, setData] = useState<NoticeUnread>({ total: 0, list: [] });
  const [loading, setLoading] = useState(true);
  const [marking, setMarking] = useState<string | null>(null);
  const [viewing, setViewing] = useState<Notice | null>(null);

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

  /** 详情取到后才标记已读：拉取失败时条目留在未读里，用户能重试 */
  const openFull = async (n: NoticeSummary) => {
    setMarking(n.id);
    try {
      const detail = await noticeApi.detail(n.id);
      setViewing(detail);
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
            {n.preview && (
              <Typography.Paragraph type="secondary" style={{ margin: '4px 0', whiteSpace: 'pre-wrap' }}>
                {n.preview}
              </Typography.Paragraph>
            )}
            <Button
              size="small"
              type="link"
              style={{ padding: 0 }}
              loading={marking === n.id}
              onClick={() => void openFull(n)}
            >
              {t('notice.view_full')}
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
    <>
      <Popover content={content} title={t('notice.unread_title', { n: data.total })} trigger="click" placement="bottomRight">
        <Badge count={data.total} size="small" overflowCount={99} offset={[-2, 2]}>
          <BellOutlined style={{ fontSize: 18, cursor: 'pointer' }} />
        </Badge>
      </Popover>
      <Modal
        open={viewing !== null}
        title={viewing?.title}
        footer={null}
        onCancel={() => setViewing(null)}
        width={520}
        destroyOnHidden
      >
        <Typography.Text type="secondary" style={{ fontSize: 12 }}>{viewing?.published_at ?? ''}</Typography.Text>
        {/* 正文按纯文本渲染（不解析 HTML） */}
        <Typography.Paragraph style={{ marginTop: 12, whiteSpace: 'pre-wrap', maxHeight: 400, overflow: 'auto' }}>
          {viewing?.content}
        </Typography.Paragraph>
      </Modal>
    </>
  );
}
