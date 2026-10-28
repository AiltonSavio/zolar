import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { readConfig } from './config';
import './styles.css';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App config={readConfig()} />
  </StrictMode>,
);
