import assert from 'node:assert/strict';
import { readFile, stat } from 'node:fs/promises';
import { resolve } from 'node:path';
import test from 'node:test';
import { pathToFileURL } from 'node:url';

const { product } = await import(pathToFileURL(resolve('src/product.ts')).href);
const origin = `https://${product.id}.chuo.me`;
const read = (path) => readFile(resolve('dist', path), 'utf8');
const decode = (text) => text.replaceAll('&amp;', '&').replaceAll('&quot;', '"').replaceAll('&#39;', "'");
const attributes = (text) => Object.fromEntries([...text.matchAll(/([\w:-]+)="([^"]*)"/g)].map(([, name, value]) => [name, decode(value)]));
const tags = (html, name) => [...html.matchAll(new RegExp(`<${name}\\b([^>]*)>`, 'g'))].map(([, text]) => attributes(text));

for (const [locale, file, path, language] of [['en', 'index.html', '/', 'en'], ['zh', 'zh/index.html', '/zh/', 'zh-CN']]) {
  const html = await read(file);
  const links = tags(html, 'link');
  const meta = (name) => tags(html, 'meta').find((item) => item.name === name || item.property === name)?.content;

  test(`${locale}: independent identity, search, social, and language metadata agree`, () => {
    assert.ok(html.includes(`<html lang="${language}">`));
    assert.equal(links.find((item) => item.rel === 'canonical')?.href, origin + path);
    assert.equal(meta('og:url'), origin + path);
    assert.equal(meta('og:title'), product.copy[locale].title);
    assert.equal(meta('twitter:title'), product.copy[locale].title);
    assert.equal(meta('description'), product.copy[locale].description);
    assert.equal(meta('og:description'), product.copy[locale].description);
    assert.equal(meta('twitter:description'), product.copy[locale].description);
    assert.equal(meta('og:image'), origin + product.hero.src);
    const alternatives = links.filter((item) => item.rel === 'alternate');
    assert.deepEqual(alternatives.map(({ hreflang, href }) => [hreflang, href]), [
      ['en', origin + '/'], ['zh-CN', origin + '/zh/'], ['x-default', origin + '/'],
    ]);
    assert.ok(!meta('robots')?.includes('noindex'));
  });

  test(`${locale}: download and source actions are available in static HTML`, () => {
    const anchors = tags(html, 'a');
    assert.equal(anchors.filter((item) => item.href === (product.release?.download ?? product.repository)).length, 2);
    assert.ok(anchors.some((item) => item.href === product.repository));
    assert.ok(anchors.some((item) => item.href === product.repository + '/releases'));
    assert.ok(anchors.some((item) => item.href === (locale === 'en' ? '/zh/' : '/')));
    if (product.release) {
      assert.ok(anchors.some((item) => item.href === product.release.url));
      assert.match(product.release.download, new RegExp(`^${product.repository}/releases/download/`));
      assert.ok(product.release.download.includes(product.release.tag));
      assert.ok(product.release.download.endsWith('arm64.dmg'));
      assert.ok(html.includes(locale === 'en' ? 'preview release' : '当前提供预览版'));
    } else {
      assert.ok(!anchors.some((item) => item.href.includes('/releases/download/')));
      assert.ok(html.includes(locale === 'en' ? 'A public download is not available yet' : '尚无公开安装包'));
    }
    assert.ok(html.includes('Apple Silicon'));
    assert.ok(html.includes('macOS 14'));
    assert.equal(tags(html, 'script').filter((item) => item.type !== 'application/ld+json').length, 0);
    assert.equal(tags(html, 'style').length, 0, 'Styles must stay external to work with the shipped self-only CSP');
    assert.equal(tags(html, 'button').length, 0, 'Static screenshot controls must not become fake website buttons');
  });

  test(`${locale}: linked assets, language pages, and section anchors exist`, async () => {
    const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map(([, id]) => id);
    assert.equal(ids.length, new Set(ids).size, 'Page IDs must be unique');
    assert.equal(tags(html, 'h1').length, 1);
    assert.equal(tags(html, 'main').length, 1);
    const references = [...tags(html, 'a').map((tag) => tag.href), ...tags(html, 'img').map((tag) => tag.src), ...links.map((tag) => tag.href)];
    for (const reference of references) {
      if (reference.startsWith('#')) assert.ok(ids.includes(reference.slice(1)), `Missing anchor: ${reference}`);
      if (!reference.startsWith('/')) continue;
      const resource = reference.endsWith('/') ? reference + 'index.html' : reference;
      assert.ok((await stat(resolve('dist', resource.slice(1)))).isFile(), `Missing resource: ${reference}`);
    }
    for (const image of tags(html, 'img')) {
      assert.ok('alt' in image);
      assert.ok(Number(image.width) > 0 && Number(image.height) > 0, 'Images must reserve layout space');
    }
    assert.equal(tags(html, 'details').length, product.copy[locale].faq.length);
  });

  test(`${locale}: structured app data describes the visible product and its release`, () => {
    const raw = html.match(/<script\b[^>]*type="application\/ld\+json"[^>]*>(.*?)<\/script>/s)?.[1];
    const data = JSON.parse(raw);
    assert.equal(data['@type'], 'SoftwareApplication');
    assert.equal(data.name, product.name);
    assert.equal(data.url, origin + path);
    assert.equal(data.inLanguage, language);
    assert.equal(data.downloadUrl, product.release?.download);
    assert.equal(data.softwareVersion, product.release?.tag);
    assert.equal(data.sameAs, product.repository);
    assert.ok(!('aggregateRating' in data));
    assert.ok(!('offers' in data));
  });
}

test('crawl resources include exactly the two localized production pages', async () => {
  const sitemap = await read('sitemap.xml');
  assert.deepEqual([...sitemap.matchAll(/<loc>([^<]+)<\/loc>/g)].map(([, url]) => url), [origin + '/', origin + '/zh/']);
  assert.equal((sitemap.match(/hreflang="zh-CN"/g) ?? []).length, 2);
  assert.match(await read('robots.txt'), /User-agent: \*\s+Allow: \//);
  assert.ok((await read('robots.txt')).includes(`Sitemap: ${origin}/sitemap.xml`));
  const missing = await read('404.html');
  assert.equal(tags(missing, 'style').length, 0, 'The 404 page must also respect the self-only style policy');
  assert.ok(tags(missing, 'meta').some((tag) => tag.name === 'robots' && tag.content === 'noindex'));
  assert.ok(tags(missing, 'a').some((tag) => tag.href === '/zh/'));
});

test('production owns only its intended domain and test keeps a separate Worker', async () => {
  const config = JSON.parse(await readFile('wrangler.jsonc', 'utf8'));
  assert.deepEqual(config.env.production.routes, [{ pattern: `${product.id}.chuo.me`, custom_domain: true }]);
  assert.notEqual(config.env.test.name, config.env.production.name);
  assert.equal(config.env.test.routes, undefined);
  assert.equal(config.env.production.assets.not_found_handling, '404-page');
  assert.equal(config.env.production.workers_dev, false);
});
