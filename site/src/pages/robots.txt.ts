import type { APIRoute } from 'astro';
import { robots } from '../site-kit/crawl';

export const GET: APIRoute = ({ site }) => robots(site!);
