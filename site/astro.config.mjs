import { defineConfig } from 'astro/config';

export default defineConfig({
  site: 'https://musicx.chuo.me',
  output: 'static',
  trailingSlash: 'always',
  build: { inlineStylesheets: 'never' },
});
