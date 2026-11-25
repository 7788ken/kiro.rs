import { useQuery } from '@tanstack/react-query'
import { DEFAULT_SITE_NAME, getBranding } from '@/api/branding'

export function useBranding() {
  const query = useQuery({
    queryKey: ['branding'],
    queryFn: getBranding,
    staleTime: Infinity,
    retry: false,
  })
  return {
    siteName: query.data?.siteName || DEFAULT_SITE_NAME,
    logoDataUrl: query.data?.logoDataUrl || null,
  }
}
