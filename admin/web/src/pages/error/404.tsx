import { Result, Button } from 'antd';
import { useNavigate } from 'react-router';

export default function NotFound() {
  const nav = useNavigate();
  return (
    <Result
      status="404"
      title="404"
      subTitle="页面不存在"
      extra={<Button type="primary" onClick={() => nav('/')}>返回首页</Button>}
    />
  );
}
