import { useState, useEffect, useCallback } from 'react';

const API_BASE_URL = process.env.NEXT_PUBLIC_API_URL || 'https://localhost:44342';

const getAuthHeaders = () => {
  const token = typeof window !== 'undefined' ? localStorage.getItem('dosi-token') : null;
  return {
    'Content-Type': 'application/json',
    ...(token ? { 'Authorization': `Bearer ${token}` } : {})
  };
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
    } catch (err: any) {
      setState({ data: null, error: err, isLoading: false });
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
    throw new Error(`API GET Error: ${res.statusText}`);
  }
  const json = await res.json();
  return json.items || json;
};

export const postApi = async (endpoint: string, body: any) => {
  const res = await fetch(`${API_BASE_URL}${endpoint}`, {
    method: 'POST',
    headers: getAuthHeaders(),
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    throw new Error(`API POST Error: ${res.statusText}`);
  }
  return await res.json();
};

export const putApi = async (endpoint: string, body: any) => {
  const res = await fetch(`${API_BASE_URL}${endpoint}`, {
    method: 'PUT',
    headers: getAuthHeaders(),
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    throw new Error(`API PUT Error: ${res.statusText}`);
  }
  return await res.json();
};

export const loginApi = async (username: string, password: string): Promise<string> => {
  const params = new URLSearchParams();
  params.append('grant_type', 'password');
  params.append('username', username);
  params.append('password', password);
  params.append('client_id', 'Tracker_App');

  const res = await fetch(`${API_BASE_URL}/connect/token`, {
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
  }
  return token;
};

export const logoutApi = () => {
  if (typeof window !== 'undefined') {
    localStorage.removeItem('dosi-token');
  }
};

export const registerApi = async (email: string, password: string, appName: string) => {
  const res = await fetch(`${API_BASE_URL}/api/account/register`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({
      userName: email,
      emailAddress: email,
      password: password,
      appName: appName
    }),
  });

  if (!res.ok) {
    const errorText = await res.text();
    throw new Error(`API Register Error: ${errorText}`);
  }
  return await res.json();
};
