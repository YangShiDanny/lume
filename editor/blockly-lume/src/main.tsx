import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';
import './lume/cursor';
import './styles/global.css';
import './styles/cursor.css';

const root = document.getElementById('root');
if (!root) throw new Error('#root not found');

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
