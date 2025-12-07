declare global {
  interface Window {
    SERVER_CONFIG?: {
      apiUrl?: string;
    };
  }
}

export const API_BASE_URL = window.SERVER_CONFIG?.apiUrl || import.meta.env.VITE_API_URL || 'http://localhost:3000';
