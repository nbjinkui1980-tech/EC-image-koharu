'use client'

import {
  useMutation,
  useQuery,
  type QueryKey,
  type UseMutationOptions,
  type UseQueryOptions,
} from '@tanstack/react-query'

import {
  deleteProject,
  getCatalog,
  getCodexAuthStatus,
  getCurrentLlm,
  getGoogleFontsCatalog,
  getMeta,
  getSceneJson,
  importProject as importProjectRequest,
  listFonts,
  listProjects,
} from './generated'
import type {
  CodexAuthStatus,
  GoogleFontCatalog,
  LlmCatalog,
  LlmState,
  MetaInfo,
  SceneSnapshot,
} from './schemas'

export {
  applyCommand,
  cancelOperation,
  createPages,
  createPagesFromPaths,
  createProject,
  deleteCodexSession,
  deleteCurrentLlm,
  deleteCurrentProject,
  fetchGoogleFont,
  getBlob,
  getCatalog,
  getConfig,
  getEngineCatalog,
  getExportCurrentProjectUrl,
  getGetGoogleFontFileUrl,
  getGetPageThumbnailUrl,
  getMeta,
  patchConfig,
  putCurrentLlm,
  putCurrentProject,
  redo,
  reorderTextNodes,
  startCodexDeviceLogin,
  startCodexImageGeneration,
  startPipeline,
  undo,
} from './generated'

const key = (path: string) => [path] as const

export const getGetCodexAuthStatusQueryKey = () => key('/api/v1/ai/codex/auth/status')
export const getGetConfigQueryKey = () => key('/api/v1/config')
export const getListFontsQueryKey = () => key('/api/v1/fonts')
export const getGetGoogleFontsCatalogQueryKey = () => key('/api/v1/google-fonts')
export const getGetCatalogQueryKey = () => key('/api/v1/llm/catalog')
export const getGetCurrentLlmQueryKey = () => key('/api/v1/llm/current')
export const getGetMetaQueryKey = () => key('/api/v1/meta')
export const getListProjectsQueryKey = () => key('/api/v1/projects')
export const getGetSceneJsonQueryKey = () => key('/api/v1/scene.json')

type ApiQueryOptions<TData> = {
  query?: Partial<UseQueryOptions<TData, unknown, TData, QueryKey>>
  request?: RequestInit
}

function useApiQuery<TData>(
  defaultQueryKey: QueryKey,
  request: (options?: RequestInit) => Promise<TData>,
  options?: ApiQueryOptions<TData>,
) {
  const { queryKey = defaultQueryKey, ...queryOptions } = options?.query ?? {}
  return useQuery<TData, unknown, TData, QueryKey>({
    queryKey,
    queryFn: ({ signal }) => request({ signal, ...options?.request }),
    gcTime: 5 * 60 * 1000,
    retry: 1,
    ...queryOptions,
  })
}

export const useGetCodexAuthStatus = (options?: ApiQueryOptions<CodexAuthStatus>) =>
  useApiQuery(getGetCodexAuthStatusQueryKey(), getCodexAuthStatus, options)

export const useGetMeta = (options?: ApiQueryOptions<MetaInfo>) =>
  useApiQuery(getGetMetaQueryKey(), getMeta, options)

export const useGetSceneJson = (options?: ApiQueryOptions<SceneSnapshot>) =>
  useApiQuery(getGetSceneJsonQueryKey(), getSceneJson, options)

export const useListProjects = (
  options?: ApiQueryOptions<Awaited<ReturnType<typeof listProjects>>>,
) => useApiQuery(getListProjectsQueryKey(), listProjects, options)

export const useGetGoogleFontsCatalog = (options?: ApiQueryOptions<GoogleFontCatalog>) =>
  useApiQuery(getGetGoogleFontsCatalogQueryKey(), getGoogleFontsCatalog, options)

export const useListFonts = (options?: ApiQueryOptions<Awaited<ReturnType<typeof listFonts>>>) =>
  useApiQuery(getListFontsQueryKey(), listFonts, options)

export const useGetCurrentLlm = (options?: ApiQueryOptions<LlmState>) =>
  useApiQuery(getGetCurrentLlmQueryKey(), getCurrentLlm, options)

export const useGetCatalog = (options?: ApiQueryOptions<LlmCatalog>) =>
  useApiQuery(getGetCatalogQueryKey(), getCatalog, options)

type DeleteProjectVariables = { id: string }
type DeleteProjectOptions = {
  mutation?: UseMutationOptions<void, unknown, DeleteProjectVariables>
  request?: RequestInit
}

export const useDeleteProject = (options?: DeleteProjectOptions) =>
  useMutation<void, unknown, DeleteProjectVariables>({
    mutationKey: ['deleteProject'],
    mutationFn: ({ id }) => deleteProject(id, options?.request),
    ...options?.mutation,
  })

/** Preserve the former facade signature used by archive upload helpers. */
export async function importProject(options?: RequestInit) {
  const { body, ...requestOptions } = options ?? {}
  const archive =
    body == null ? undefined : body instanceof Blob ? body : new Blob([body as unknown as BlobPart])
  return importProjectRequest(archive, requestOptions)
}
