import { Result, Button } from 'antd';
import { useNavigate } from 'react-router';

export default function NotFound() {
  const nav = useNavigate();
  return (
    <Result
      icon={<img src="/keeper-verifying.svg" alt="阿守：举着放大镜，没找到这个页面" width={150} height={150} />}
      title="404"
      subTitle="页面不存在"
      extra={<Button type="primary" onClick={() => nav('/')}>返回首页</Button>}
    />
  );
}
