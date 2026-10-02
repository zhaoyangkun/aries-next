<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type Vditor from 'vditor'
import { LoaderCircleIcon } from '@lucide/vue'
import { MEDIA_MAX_FILE_SIZE, MEDIA_UPLOAD_ACCEPT } from '@/modules/media/api/media'

// 通用 Markdown 编辑器（Vditor 即时渲染模式），供页面/日志等轻量编辑场景复用。
// 文章编辑器的自动保存与备份逻辑仍由 ArticleEditorDialog 自持，本组件不做持久化。

const props = withDefaults(defineProps<{
  modelValue: string
  height?: number
  placeholder?: string
  // 图片上传 handler 由调用方注入（通常走 mediaApi.upload）；返回字符串即视为错误提示。
  upload?: (files: File[], insert: (markdown: string) => void) => Promise<string | null>
  counterMax?: number
}>(), {
  height: 420,
  placeholder: '使用 Markdown 编写内容...',
  upload: undefined,
  counterMax: 100000,
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const containerEl = ref<HTMLElement | null>(null)
const ready = ref(false)
const loadError = ref('')
let vditorInstance: Vditor | null = null
// 程序化 setValue 也会触发 input 回调，用内部快照避免回环。
let internalValue = props.modelValue

onMounted(() => {
  void createVditor()
})

onBeforeUnmount(() => {
  destroyVditor()
})

// 外部重置内容（如切换编辑对象）时同步进编辑器。
watch(
  () => props.modelValue,
  (value) => {
    if (value === internalValue) return
    internalValue = value
    vditorInstance?.setValue(value)
  },
)

// Vditor 体积较大，动态加载避免拖慢首屏；CSS 一并按需引入。
async function createVditor() {
  if (!containerEl.value || vditorInstance) return
  try {
    const [{ default: VditorCtor }] = await Promise.all([
      import('vditor'),
      import('vditor/dist/index.css'),
    ])
    // 加载期间组件可能已卸载（Dialog 被关闭），此时放弃创建。
    if (!containerEl.value || vditorInstance) return
    vditorInstance = new VditorCtor(containerEl.value, {
      mode: 'ir',
      // 运行期资产（lute / hljs / KaTeX / icons 等）走 npmmirror（淘宝 npm 镜像，
      // 国内访问最快且稳定）；默认 unpkg 国内超时会卡编辑器加载。
      // 注意：升级 vditor 依赖时需同步下面的版本号。
      cdn: 'https://registry.npmmirror.com/vditor/3.11.3/files',
      height: props.height,
      toolbarConfig: { pin: true },
      cache: { enable: false },
      counter: { enable: true, max: props.counterMax },
      placeholder: props.placeholder,
      value: props.modelValue,
      input: (value) => {
        internalValue = value
        emit('update:modelValue', value)
      },
      ...(props.upload
        ? {
            upload: {
              accept: MEDIA_UPLOAD_ACCEPT,
              multiple: true,
              max: MEDIA_MAX_FILE_SIZE,
              fieldName: 'file[]',
              // Vditor 类型未声明 Promise<string | null>，此处收窄为联合类型。
              handler: (files: File[]) =>
                props.upload!(files, (markdown) => {
                  vditorInstance?.insertValue(markdown)
                }) as Promise<string> | Promise<null>,
            },
          }
        : {}),
    })
    ready.value = true
  }
  catch {
    loadError.value = '编辑器加载失败，请刷新重试'
  }
}

function destroyVditor() {
  if (!vditorInstance) return
  try {
    vditorInstance.destroy()
  }
  catch {
    // 销毁阶段的内部异常不影响组件卸载。
  }
  vditorInstance = null
}
</script>

<template>
  <div class="relative min-w-0">
    <div
      v-if="!ready && !loadError"
      class="flex items-center gap-1.5 py-3 text-xs text-muted-foreground"
      role="status"
    >
      <LoaderCircleIcon class="size-3.5 animate-spin" />
      正在加载编辑器
    </div>
    <p v-if="loadError" role="alert" class="py-3 text-xs font-medium text-destructive">{{ loadError }}</p>
    <div ref="containerEl" class="w-full" />
  </div>
</template>
