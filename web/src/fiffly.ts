// fiffly, the browser video editor: links that open media in it, and the
// endpoints behind Settings' fiffly section and the consent page.
import { useQuery } from '@tanstack/react-query';
import { api, getSettings, type Settings } from './api';

export type Connection = {
  id: number;
  client: string;
  scopes: string[];
  createdAt: string;
  lastUsed: string | null;
};

export type AuthorizeInfo = { client: string; scopes: string[]; redirectHost: string };

export const listConnections = () => api.get<{ connections: Connection[] }>('/api/fiffly/connections');
export const revokeConnection = (id: number) => api.del<{ ok: boolean }>(`/api/fiffly/connections/${id}`);
export const checkAuthorize = (search: string) => api.get<AuthorizeInfo>(`/api/oauth/authorize${search}`);
export const answerAuthorize = (params: Record<string, string>, approve: boolean) =>
  api.post<{ redirectTo: string }>('/api/oauth/authorize', { ...params, approve });
const fifflySlug = (id: number) => api.get<{ slug: string }>(`/api/media/${id}/fiffly`);

const isLoopback = (host: string) => /^(localhost|127\.[\d.]+|\[::1\])(:\d+)?$/.test(host);

export type FifflyTarget = {
  /** fiffly's origin. */
  fiffly: string;
  /** hoard's origin, as fiffly reaches it. */
  site: string;
  host: string;
  /** Why fiffly will refuse that address, if it will. */
  problem: string | null;
};

/**
 * fiffly runs in the browser, so it reaches hoard at whatever address the
 * browser does — unless Settings names another. It insists on https, except
 * for this computer itself.
 */
export function fifflyTarget(settings: Settings | undefined): FifflyTarget {
  const fiffly = (settings?.fifflyUrl || settings?.defaultFifflyUrl || 'https://fiffly.com').replace(/\/+$/, '');
  const site = settings?.fifflySite || window.location.origin;
  const { protocol, host } = new URL(site);
  const problem =
    protocol === 'http:' && !isLoopback(host)
      ? `fiffly only talks to sites on https, or on this computer (localhost) — ${host} is neither.`
      : null;
  return { fiffly, site, host, problem };
}

export function useFiffly(): FifflyTarget {
  const { data } = useQuery({ queryKey: ['settings'], queryFn: getSettings, staleTime: 60_000 });
  return fifflyTarget(data);
}

/** The page on fiffly that offers to add hoard as a site. */
export const addSiteUrl = (t: FifflyTarget) => `${t.fiffly}/?add-site=${encodeURIComponent(t.site)}`;

/**
 * A link opening the media in fiffly's editor. It carries a slug signed for a
 * few hours, so it works before hoard is connected in fiffly at all. Fetched
 * only while `enabled`, so a grid does not sign a link per tile.
 */
export function useFifflyLink(id: number, enabled = true): string | undefined {
  const target = useFiffly();
  const { data } = useQuery({
    queryKey: ['fiffly-slug', id],
    queryFn: () => fifflySlug(id),
    enabled,
    staleTime: 60 * 60_000,
  });
  if (!data) return undefined;
  return `${target.fiffly}/remix?site=${encodeURIComponent(target.host)}&media=${encodeURIComponent(data.slug)}`;
}
