import type { APIRoute } from 'astro';
import { sitemap } from '../site-kit/crawl';

export const GET: APIRoute = ({ site }) => sitemap(site!);
