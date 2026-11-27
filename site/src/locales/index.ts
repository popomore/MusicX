import { messages as en } from './en.ts';
import { messages as zh } from './zh-CN.ts';
import type { Locale } from '../site-kit/types';

export const messages: Record<Locale, typeof en> = { en, zh };
export const productCopy = { en: en.copy, zh: zh.copy };
