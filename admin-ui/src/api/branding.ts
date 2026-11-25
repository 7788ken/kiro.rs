import axios from 'axios'

export interface Branding {
  siteName: string | null
  logoDataUrl: string | null
}

export const DEFAULT_SITE_NAME = 'Kiro Console'

// 公开接口，不附带鉴权头
export async function getBranding(): Promise<Branding> {
  const { data } = await axios.get<Branding>('/api/branding')
  return data
}
