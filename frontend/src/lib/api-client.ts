// lib/api-client.ts

const API_BASE_URL = process.env.NEXT_PUBLIC_API_URL || 'https://localhost:44342';

// Helper to get token if auth is implemented
const getAuthHeaders = () => {
  // In a real app, you'd fetch the token from NextAuth or localStorage
  // For now, we mock the header if not authenticated or bypass
  const token = typeof window !== 'undefined' ? localStorage.getItem('dosi-token') : null;
  return {
    'Content-Type': 'application/json',
    ...(token ? { 'Authorization': `Bearer ${token}` } : {})
  };
};

export const fetchProjects = async () => {
  try {
    const res = await fetch(`${API_BASE_URL}/api/app/project`, {
      headers: getAuthHeaders()
    });
    if (!res.ok) throw new Error('Failed to fetch projects');
    const data = await res.json();
    return data.items;
  } catch (error) {
    console.error(error);
    return [];
  }
};

export const createProject = async (projectData: any) => {
  try {
    const res = await fetch(`${API_BASE_URL}/api/app/project`, {
      method: 'POST',
      headers: getAuthHeaders(),
      body: JSON.stringify(projectData)
    });
    if (!res.ok) throw new Error('Failed to create project');
    return await res.json();
  } catch (error) {
    console.error(error);
    throw error;
  }
};

export const fetchActivities = async () => {
  try {
    const res = await fetch(`${API_BASE_URL}/api/app/activity`, {
      headers: getAuthHeaders()
    });
    if (!res.ok) throw new Error('Failed to fetch activities');
    const data = await res.json();
    return data.items;
  } catch (error) {
    console.error(error);
    return [];
  }
};
