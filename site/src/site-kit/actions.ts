import type { Locale, Product } from './types';

export function productActions(product: Product, locale: Locale) {
  const chinese = locale === 'zh';
  const ui = chinese ? {
    get: '下载预览版', source: '查看源码', releases: '所有版本', guide: '使用指南',
    requirements: 'macOS 14 及以上 · Apple Silicon', image: '查看完整截图',
    unavailable: '当前提供源码，尚无公开安装包。',
    available: '当前提供预览版。下载由 GitHub Releases 提供。',
    notes: '此版本说明', license: 'MIT 开源', back: '返回顶部',
  } : {
    get: 'Download preview', source: 'View source', releases: 'All releases', guide: 'Getting started',
    requirements: 'macOS 14 or later · Apple Silicon', image: 'View full screenshot',
    unavailable: 'Source is available. A public download is not available yet.',
    available: 'A preview release, available from GitHub Releases.',
    notes: 'Release notes', license: 'MIT open source', back: 'Back to top',
  };
  return {
    ...ui,
    home: chinese ? '/zh/' : '/',
    primaryHref: product.release?.download ?? product.repository,
    primaryLabel: product.release ? ui.get : ui.source,
    secondaryHref: product.release ? product.repository : `${product.repository}/releases`,
    secondaryLabel: product.release ? ui.source : ui.releases,
    availability: product.release ? ui.available : ui.unavailable,
  };
}
