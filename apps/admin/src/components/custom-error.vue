<script setup lang="ts">
/** 错误页展示组件：错误码、副标题与描述文案。Props: code/subtitle/error。 */
import { Button } from '@/components/ui/button'

defineProps<{
  code: number
  subtitle: string
  error: string
}>()
</script>

<template>
  <div class="max-w-2xl mx-auto text-center">
    <h1 class="font-bold text-8xl">
      {{ code }}
    </h1>
    <h2 class="mt-4 text-2xl font-bold">
      {{ subtitle }}
    </h2>
    <p class="text-stone-400">
      {{ error }}
    </p>

    <footer class="mt-8">
      <slot>
        <div class="flex justify-center gap-2">
          <Button variant="outline" @click="$router.back()">
            返回上页
          </Button>
          <Button @click="$router.push('/')">
            回到首页
          </Button>
        </div>
      </slot>
    </footer>
  </div>
</template>
