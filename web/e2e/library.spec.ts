// The UI against the real server: browsing, search, tagging, the viewer and
// the fiffly entry points. The library is red.mp4 in clips/, blue.mp4 and
// photo.png (e2e/serve.mjs).
import { expect, test, type Page } from '@playwright/test';

type Item = { id: number; filename: string; thumbUrl: string | null };

test.beforeAll(async ({ request }) => {
  // The first scan, probing and thumbnails run in the background.
  await expect
    .poll(
      async () => {
        const { media } = (await (await request.get('/api/media?limit=50')).json()) as { media: Item[] };
        return media.filter((m) => m.thumbUrl).length;
      },
      { timeout: 30_000 },
    )
    .toBe(3);
});

const tile = (page: Page, title: string) => page.locator('div.group', { has: page.locator(`a[title="${title}"]`) });

async function idOf(page: Page, filename: string): Promise<number> {
  const { media } = (await (await page.request.get('/api/media?limit=50')).json()) as { media: Item[] };
  return media.find((m) => m.filename === filename)!.id;
}

test('the library lists media with thumbnails, and folders narrow it', async ({ page }) => {
  await page.goto('/library');
  for (const title of ['red', 'blue', 'photo']) {
    await expect(tile(page, title).locator('img').first()).toBeVisible();
  }
  await expect(tile(page, 'red')).toContainText('0:03');

  await page.getByRole('link', { name: 'library' }).first().click();
  await page.getByRole('link', { name: /^clips/ }).first().click();
  await expect(tile(page, 'red')).toBeVisible();
  await expect(tile(page, 'blue')).toHaveCount(0);
});

test('the search box finds media as you type and runs a full search on Enter', async ({ page }) => {
  await page.goto('/library');
  await expect(tile(page, 'blue')).toBeVisible();
  // "/" focuses the box from anywhere on the page.
  await page.keyboard.press('/');
  await expect(page.getByRole('combobox', { name: /Search/ })).toBeFocused();
  await page.keyboard.type('blue');
  await expect(page.getByRole('option').filter({ hasText: 'blue' }).first()).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/search\?q=blue/);
  await expect(tile(page, 'blue')).toBeVisible();
  await expect(tile(page, 'red')).toHaveCount(0);
});

test('tags added from the selection bar show on the tile and are searchable', async ({ page }) => {
  await page.goto('/library');
  await tile(page, 'red').hover();
  await page.getByRole('checkbox', { name: 'Select red' }).check();
  const input = page.getByPlaceholder('Add tags…');
  await input.fill('sunset');
  await input.press('Enter');
  await page.getByRole('button', { name: 'Add', exact: true }).click();
  await expect(tile(page, 'red').getByRole('link', { name: 'sunset' })).toBeVisible();

  await page.goto(`/search?q=${encodeURIComponent('tag:sunset')}`);
  await expect(tile(page, 'red')).toBeVisible();
  await expect(tile(page, 'blue')).toHaveCount(0);
});

test('the viewer plays a video and closes back to the grid', async ({ page }) => {
  const id = await idOf(page, 'red.mp4');
  await page.goto('/library');
  await tile(page, 'red').locator('a[title="red"]').click();
  await expect(page).toHaveURL(new RegExp(`/m/${id}$`));
  const video = page.locator('video');
  await expect(video).toHaveAttribute('src', `/f/${id}`);
  await expect.poll(() => video.evaluate((v: HTMLVideoElement) => v.readyState), { timeout: 10_000 }).toBeGreaterThan(0);
  await page.keyboard.press('Escape');
  await expect(page).toHaveURL(/\/library$/);
});

test('media open in fiffly from the tile menu and the viewer', async ({ page }) => {
  const id = await idOf(page, 'red.mp4');
  const remix = new RegExp(`^https://fiffly\\.com/remix\\?site=127\\.0\\.0\\.1%3A\\d+&media=${id}-\\d+-[0-9a-f]{32}$`);

  await page.goto('/library');
  await tile(page, 'red').hover();
  await tile(page, 'red').getByRole('button', { name: 'More actions' }).click();
  await expect(page.getByRole('menuitem', { name: 'Open in fiffly' })).toHaveAttribute('href', remix);
  await expect(page.getByRole('menuitem', { name: 'Download' })).toHaveAttribute('href', `/f/${id}`);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menu')).toHaveCount(0);

  await page.goto(`/m/${id}`);
  await expect(page.getByRole('link', { name: 'Open in fiffly' })).toHaveAttribute('href', remix);

  // The link's slug opens the post without a connected account.
  const href = (await page.getByRole('link', { name: 'Open in fiffly' }).getAttribute('href'))!;
  const slug = new URL(href).searchParams.get('media')!;
  const post = await page.request.get(`/api/publish/media/${slug}`);
  expect(post.ok()).toBe(true);
  expect((await post.json()).title).toBe('red');
});

test('settings link to fiffly, and the consent page hands fiffly a code', async ({ page, baseURL }) => {
  await page.goto('/settings');
  await expect(page.getByRole('link', { name: 'Add hoard to fiffly' })).toHaveAttribute(
    'href',
    `https://fiffly.com/?add-site=${encodeURIComponent(baseURL!)}`,
  );
  await expect(page.getByText('Nothing connected yet.')).toBeVisible();

  // Where fiffly would take over; nothing leaves the machine.
  await page.route('https://fiffly.com/**', (route) => route.fulfill({ body: 'fiffly' }));
  const redirect = 'https://fiffly.com/publish/callback/127.0.0.1';
  const params = new URLSearchParams({
    response_type: 'code',
    client_id: 'fiffly',
    redirect_uri: redirect,
    scope: 'media:read media:write',
    state: 'xyz',
    code_challenge: 'E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM',
    code_challenge_method: 'S256',
  });
  await page.goto(`/oauth/authorize?${params}`);
  await expect(page.getByRole('heading', { name: 'Connect fiffly to hoard?' })).toBeVisible();
  await page.getByRole('button', { name: 'Allow' }).click();
  await expect(page).toHaveURL(new RegExp(`^${redirect}\\?code=[0-9a-f]+&state=xyz$`));

  await page.goto(`/oauth/authorize?${params.toString().replace('fiffly.com', 'evil.example')}`);
  await expect(page.getByText('This request can’t be shown')).toBeVisible();
});

// Last: it reorganises the library the tests above rely on.
test('folders move by dragging and from the sidebar', async ({ page }) => {
  const { roots } = (await (await page.request.get('/api/roots')).json()) as { roots: { id: number }[] };
  const rootId = roots[0].id;
  for (const path of ['archive', 'later']) {
    expect((await page.request.post('/api/folders', { data: { rootId, path } })).ok()).toBe(true);
  }
  const folderOf = async (filename: string) => {
    const id = await idOf(page, filename);
    return ((await (await page.request.get(`/api/media/${id}`)).json()) as { folder: string }).folder;
  };

  // Drag the clips tile onto the archive tile.
  await page.goto(`/library?root=${rootId}`);
  page.once('dialog', (d) => void d.accept());
  const grid = page.locator('main');
  await grid.getByRole('link', { name: /^clips/ }).dragTo(grid.getByRole('link', { name: /^archive/ }));
  await expect.poll(() => folderOf('red.mp4'), { timeout: 15_000 }).toBe('archive/clips');

  // Then from the sidebar's folder menu, while viewing it: the page follows.
  await page.goto(`/library?root=${rootId}&folder=archive%2Fclips`);
  const node = page.locator('aside div.group\\/node', { has: page.getByRole('link', { name: 'clips', exact: true }) });
  await node.hover();
  await node.getByRole('button', { name: 'Move folder' }).click();
  page.once('dialog', (d) => void d.accept());
  const input = page.getByPlaceholder('Move clips into…');
  await input.fill('library/later');
  await input.press('Enter');
  await expect(page).toHaveURL(/folder=later%2Fclips/);
  await expect.poll(() => folderOf('red.mp4'), { timeout: 15_000 }).toBe('later/clips');
  await expect(tile(page, 'red')).toBeVisible();
});
