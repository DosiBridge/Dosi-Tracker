import { useState, useEffect, useCallback } from 'react';

const API_BASE_URL = process.env.NEXT_PUBLIC_API_URL || 'https://localhost:44342';

const getAuthHeaders = () => {
  const token = typeof window !== 'undefined' ? localStorage.getItem('dosi-token') : null;
  return {
    'Content-Type': 'application/json',
    ...(token ? { 'Authorization': `Bearer ${token}` } : {})
  };
};

/** Expired/revoked session: clear it and send the user back to /login. */
const handleUnauthorized = (res: Response) => {
  if (res.status === 401 && typeof window !== 'undefined') {
    logoutApi();
    if (!window.location.pathname.startsWith('/login')) {
      window.location.href = '/login';
    }
  }
};

interface ApiState<T> {
  data: T | null;
  error: Error | null;
  isLoading: boolean;
}

export function useApi<T>(endpoint: string) {
  const [state, setState] = useState<ApiState<T>>({
    data: null,
    error: null,
    isLoading: true,
  });

  const fetchData = useCallback(async () => {
    setState(prev => ({ ...prev, isLoading: true, error: null }));
    try {
      const res = await fetch(`${API_BASE_URL}${endpoint}`, {
        headers: getAuthHeaders(),
      });
      if (!res.ok) {
        throw new Error(`API Error: ${res.statusText}`);
      }
      const json = await res.json();
      setState({ data: json.items || json, error: null, isLoading: false });
    } catch (err) {
      setState({
        data: null,
        error: err instanceof Error ? err : new Error(String(err)),
        isLoading: false,
      });
    }
  }, [endpoint]);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  return { ...state, refetch: fetchData };
}

export const getApi = async (endpoint: string) => {
  const res = await fetch(`${API_BASE_URL}${endpoint}`, {
    headers: getAuthHeaders(),
  });
  if (!res.ok) {
    handleUnauthorized(res);
    throw new Error(`API GET Error: ${res.statusText}`);
  }
  const json = await res.json();
  return json.items || json;
};

/**
 * Fetch a protected binary (e.g. screenshot content) with the bearer token and
 * return an object URL for <img src>. Callers should revoke it when done.
 */
export const getAuthedBlobUrl = async (endpoint: string): Promise<string> => {
  const token = typeof window !== 'undefined' ? localStorage.getItem('dosi-token') : null;
  const res = await fetch(`${API_BASE_URL}${endpoint}`, {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
  });
  if (!res.ok) {
    handleUnauthorized(res);
    throw new Error(`API blob Error: ${res.statusText}`);
  }
  return URL.createObjectURL(await res.blob());
};

export const delApi = async (endpoint: string) => {
  const res = await fetch(`${API_BASE_URL}${endpoint}`, {
    method: 'DELETE',
    headers: getAuthHeaders(),
  });
  if (!res.ok) {
    handleUnauthorized(res);
    throw new Error(`API DELETE Error: ${res.statusText}`);
  }
};

export const postApi = async (endpoint: string, body: unknown) => {
  const res = await fetch(`${API_BASE_URL}${endpoint}`, {
    method: 'POST',
    headers: getAuthHeaders(),
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    handleUnauthorized(res);
    let detail = '';
    try {
      const parsed = JSON.parse(await res.text());
      detail = parsed?.error?.message ? `: ${parsed.error.message}` : '';
    } catch {}
    throw new Error(`API POST Error: ${res.statusText}${detail}`);
  }
  const text = await res.text();
  return text ? JSON.parse(text) : null; // 204-safe
};

export const putApi = async (endpoint: string, body: unknown) => {
  const res = await fetch(`${API_BASE_URL}${endpoint}`, {
    method: 'PUT',
    headers: getAuthHeaders(),
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    handleUnauthorized(res);
    throw new Error(`API PUT Error: ${res.statusText}`);
  }
  const text = await res.text();
  return text ? JSON.parse(text) : null; // 204-safe
};

/**
 * OpenIddict resource-owner password flow. For tenant users the token request
 * must carry the tenant (`__tenant`); afterwards the JWT itself resolves the
 * tenant on every API call.
 *
 * The token is mirrored into a cookie so the Next.js middleware can gate
 * routes server-side (localStorage is invisible to middleware).
 */
export const loginApi = async (username: string, password: string, tenant?: string): Promise<string> => {
  const params = new URLSearchParams();
  params.append('grant_type', 'password');
  params.append('username', username);
  params.append('password', password);
  params.append('client_id', 'Tracker_App');
  params.append('scope', 'Tracker');

  // The tenant MUST be a query parameter, not a form field: ABP's tenant
  // resolvers read route/query/header/cookie and ignore the POST body, so a
  // body-only `__tenant` silently resolves to the host tenant.
  const tokenUrl = tenant
    ? `${API_BASE_URL}/connect/token?__tenant=${encodeURIComponent(tenant)}`
    : `${API_BASE_URL}/connect/token`;

  const res = await fetch(tokenUrl, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/x-www-form-urlencoded',
    },
    body: params.toString(),
  });

  if (!res.ok) {
    throw new Error('Invalid credentials or login failed');
  }

  const json = await res.json();
  const token = json.access_token;
  if (typeof window !== 'undefined' && token) {
    localStorage.setItem('dosi-token', token);
    if (tenant) {
      localStorage.setItem('dosi-tenant', tenant);
    } else {
      localStorage.removeItem('dosi-tenant');
    }
    const maxAge = typeof json.expires_in === 'number' ? json.expires_in : 3600;
    const secure = window.location.protocol === 'https:' ? '; Secure' : '';
    document.cookie = `dosi-token=1; Path=/; Max-Age=${maxAge}; SameSite=Lax${secure}`;
  }
  return token;
};

export const logoutApi = () => {
  if (typeof window !== 'undefined') {
    localStorage.removeItem('dosi-token');
    localStorage.removeItem('dosi-tenant');
    document.cookie = 'dosi-token=; Path=/; Max-Age=0; SameSite=Lax';
  }
};

/**
 * Self-service SaaS signup: creates an isolated tenant with its own admin user
 * and starts a subscription on the chosen plan (backend: WorkspaceAppService).
 */
export const registerWorkspaceApi = async (
  name: string,
  adminEmail: string,
  adminPassword: string,
  planName?: string,
) => {
  const res = await fetch(`${API_BASE_URL}/api/app/workspace/register`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ name, adminEmail, adminPassword, planName }),
  });

  if (!res.ok) {
    const errorText = await res.text();
    let message = `Workspace registration failed (HTTP ${res.status})`;
    try {
      const parsed = JSON.parse(errorText);
      message = parsed?.error?.message || message;
    } catch {}
    throw new Error(message);
  }
  return await res.json() as { tenantId: string; name: string };
};
