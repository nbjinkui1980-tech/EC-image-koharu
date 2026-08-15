'use client'

import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { useEffect, useMemo, useState } from 'react'

import { getBlob } from '@/lib/api'
import { convertToBlob, revokeObjectUrlLater } from '@/lib/io/blobConvert'

const blobQueryOptions = (hash: string) => ({
  queryKey: ['blob', hash] as const,
  queryFn: async () => {
    const blob = await getBlob(hash)
    const buf = await (blob as Blob).arrayBuffer()
    return new Uint8Array(buf)
  },
  staleTime: Infinity,
  gcTime: 10 * 60 * 1000,
  structuralSharing: false as const,
})

/** Fetch blob bytes by hash. Keeps previous data as placeholder while loading. */
export function useBlobData(hash: string | undefined): Uint8Array | undefined {
  const { data } = useQuery({
    ...blobQueryOptions(hash ?? ''),
    enabled: !!hash,
    placeholderData: keepPreviousData,
  })
  return hash ? data : undefined
}

const blobImageQueryOptions = (hash: string) => ({
  queryKey: ['blobImage', hash] as const,
  queryFn: async () => {
    const response = await getBlob(hash)
    const buf = await (response as Blob).arrayBuffer()
    const bytes = new Uint8Array(buf)
    const blob = await convertToBlob(bytes)
    const preloadUrl = URL.createObjectURL(blob)
    try {
      await new Promise<void>((resolve, reject) => {
        const img = new Image()
        img.onload = () => resolve()
        img.onerror = () => reject(new Error('Failed to preload sprite'))
        img.src = preloadUrl
      })
    } finally {
      URL.revokeObjectURL(preloadUrl)
    }
    return blob
  },
  staleTime: Infinity,
  gcTime: 10 * 60 * 1000,
  structuralSharing: false as const,
})

/**
 * Fetch blob, convert to displayable format, and preload. The query cache
 * holds the Blob; each hook instance owns an object URL it creates and
 * schedules for revoke on replacement or unmount. `data` keeps the previous
 * URL while a new one loads.
 */
export function useBlobImage(hash: string | undefined) {
  const query = useQuery({
    ...blobImageQueryOptions(hash ?? ''),
    enabled: !!hash,
    placeholderData: keepPreviousData,
  })
  const blob = hash ? query.data : undefined
  const [url, setUrl] = useState<string>()
  useEffect(() => {
    if (!blob) {
      setUrl(undefined)
      return
    }
    const objectUrl = URL.createObjectURL(blob)
    setUrl(objectUrl)
    return () => revokeObjectUrlLater(objectUrl)
  }, [blob])
  return useMemo(() => ({ ...query, data: url }), [query, url])
}
