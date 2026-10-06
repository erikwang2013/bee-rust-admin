import { Result, Button } from 'antd';
import { useNavigate } from 'react-router';
import { useI18n } from '../../i18n';

export default function NotFound() {
  const nav = useNavigate();
  const { t } = useI18n();
  return (
    <Result
      icon={<img src="/keeper-verifying.svg" alt={t('error.not_found_alt')} width={150} height={150} />}
      title="404"
      subTitle={t('error.not_found_desc')}
      extra={<Button type="primary" onClick={() => nav('/')}>{t('error.back_home')}</Button>}
    />
  );
}
