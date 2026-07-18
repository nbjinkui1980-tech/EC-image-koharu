'use client'

import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import type { AppConfig, LlmProviderCatalog, TypographyPlannerConfig } from '@/lib/api/schemas'

type TypographyPlannerSettingsProps = {
  config: AppConfig
  catalog?: LlmProviderCatalog
  onChange: (config: TypographyPlannerConfig) => void
  onOpenProviders: () => void
}

export function TypographyPlannerSettings({
  config,
  catalog,
  onChange,
  onOpenProviders,
}: TypographyPlannerSettingsProps) {
  const { t } = useTranslation()
  const planner = config.typography_planner ?? {}
  const modelId = planner.model_id ?? ''
  const enabled = planner.enabled === true
  const hasBaseUrl = config.providers
    ?.find((provider) => provider.id === 'openai-compatible')
    ?.base_url?.trim()
  const models =
    catalog?.models.filter(
      ({ target }) => target.kind === 'provider' && target.providerId === 'openai-compatible',
    ) ?? []
  const catalogReady = catalog?.status === 'ready'
  const hasValidModel = models.some(({ target }) => target.modelId === modelId)
  const invalidModel = Boolean(modelId) && !hasValidModel
  const canEnable = Boolean(hasBaseUrl && catalogReady && hasValidModel)

  return (
    <div className='space-y-5'>
      <div className='space-y-1'>
        <h2 className='text-base font-semibold'>{t('settings.typography')}</h2>
        <p className='text-xs text-muted-foreground'>{t('settings.typographyDescription')}</p>
      </div>

      <div className='rounded-md border border-border bg-muted/30 p-3 text-xs text-muted-foreground'>
        <p>{t('settings.typographySharedConnection')}</p>
        <p className='mt-1'>{t('settings.typographyReloadTranslation')}</p>
      </div>

      <div className='space-y-1.5'>
        <Label className='text-xs' htmlFor='typography-planner-model'>
          {t('settings.typographyModel')}
        </Label>
        <Select
          value={hasValidModel ? modelId : ''}
          disabled={!catalogReady}
          onValueChange={(nextModelId) =>
            onChange({
              model_id: nextModelId,
              enabled: invalidModel ? false : enabled,
            })
          }
        >
          <SelectTrigger
            id='typography-planner-model'
            data-testid='typography-planner-model'
            className='w-full'
            aria-invalid={invalidModel}
          >
            <SelectValue placeholder={t('settings.typographyModelPlaceholder')} />
          </SelectTrigger>
          <SelectContent>
            {models.map(({ name, target }) => (
              <SelectItem key={target.modelId} value={target.modelId}>
                {name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {invalidModel && (
          <p data-testid='typography-planner-invalid-model' className='text-xs text-destructive'>
            {t('settings.typographyInvalidModel')}: {modelId}
          </p>
        )}
      </div>

      <label className='flex items-start gap-2 text-sm'>
        <input
          data-testid='typography-planner-enabled'
          type='checkbox'
          className='mt-0.5 size-4'
          checked={enabled}
          disabled={!enabled && !canEnable}
          onChange={(event) =>
            onChange({ model_id: modelId || null, enabled: event.target.checked })
          }
        />
        <span>
          <span className='block font-medium'>{t('settings.typographyAutoEnable')}</span>
          <span className='block text-xs text-muted-foreground'>
            {t('settings.typographyAutoEnableDescription')}
          </span>
        </span>
      </label>

      {!canEnable && (
        <div className='space-y-1'>
          <p className='text-xs text-muted-foreground'>
            {t('settings.typographyConnectionRequired')}
          </p>
          <Button
            data-testid='typography-provider-settings'
            variant='link'
            size='sm'
            className='h-auto px-0'
            onClick={onOpenProviders}
          >
            {t('settings.typographyOpenProviders')}
          </Button>
        </div>
      )}
    </div>
  )
}
