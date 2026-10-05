import { Result, Button } from 'antd';
import { useNavigate } from 'react-router';

export default function Forbidden() {
  const nav = useNavigate();
  return (
    <Result
      icon={<img src="/keeper-alarmed.svg" alt="阿守：尾针竖起，此路不通" width={150} height={150} />}
      title="403"
      subTitle="没有权限访问该页面"
      extra={<Button type="primary" onClick={() => nav('/')}>返回首页</Button>}
    />
  );
}
