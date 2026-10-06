import { Result, Button } from 'antd';
import { useNavigate } from 'react-router';
import { useI18n } from '../../i18n';

export default function Forbidden() {
  const nav = useNavigate();
  const { t } = useI18n();
  return (
    <Result
      icon={<img src="/keeper-alarmed.svg" alt={t('error.forbidden_alt')} width={150} height={150} />}
      title="403"
      subTitle={t('error.forbidden_desc')}
      extra={<Button type="primary" onClick={() => nav('/')}>{t('error.back_home')}</Button>}
    />
  );
}
