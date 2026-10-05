import { createBrowserRouter } from 'react-router';
import NotFound from './pages/error/404';
import Forbidden from './pages/error/403';

export const router = createBrowserRouter([
  { path: '/403', element: <Forbidden /> },
  { path: '*', element: <NotFound /> },
]);
