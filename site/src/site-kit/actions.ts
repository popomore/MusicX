import { messages } from '../locales/index.ts';
import type { Locale, Product } from './types';

export function productActions(product: Product, locale: Locale) {
  const chinese = locale === 'zh';
  const ui = messages[locale].actions;
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
