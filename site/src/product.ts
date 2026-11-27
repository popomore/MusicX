import { productCopy } from './locales/index.ts';
import type { Product } from './site-kit/types';

export const product = {
  id: 'musicx',
  name: 'MusicX',
  repository: 'https://github.com/popomore/MusicX',
  guide: 'https://github.com/popomore/MusicX#readme',
  release: {
    tag: 'v1.0.1.preview-20261116031314-f5d2',
    url: 'https://github.com/popomore/MusicX/releases/tag/v1.0.1.preview-20261116031314-f5d2',
    download: 'https://github.com/popomore/MusicX/releases/download/v1.0.1.preview-20261116031314-f5d2/MusicX-macos-arm64.dmg',
  },
  hero: { src: '/images/player.jpg', width: 640, height: 1024 },
  copy: productCopy,
} satisfies Product;
