import { Result, Button } from 'antd';
import { useNavigate } from 'react-router';

export default function Forbidden() {
  const nav = useNavigate();
  return (
    <Result
      status="403"
      title="403"
      subTitle="没有权限访问该页面"
      extra={<Button type="primary" onClick={() => nav('/')}>返回首页</Button>}
    />
  );
}
