import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';

async function start() {
  // Browser preview with a fake backend (development builds only): see lib/mock.ts.
  if (import.meta.env.DEV) {
    const params = new URLSearchParams(location.search);
    if (params.has('mock')) (await import('./lib/mock')).installMock(params);
  }
  return mount(App, { target: document.getElementById('app')! });
}

export default start();
