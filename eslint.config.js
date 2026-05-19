import js from '@eslint/js'
import pluginVue from 'eslint-plugin-vue'
import ts from 'typescript-eslint'
import globals from 'globals'

export default ts.config(
  js.configs.recommended,
  ...ts.configs.recommended,
  ...pluginVue.configs['flat/essential'],
  {
    files: ['src/**/*.{ts,vue}'],
    languageOptions: {
      globals: { ...globals.browser },
      parserOptions: {
        parser: ts.parser,
      },
    },
  },
  {
    files: ['src/__tests__/**'],
    rules: {
      '@typescript-eslint/no-unused-vars': 'off',
      '@typescript-eslint/no-explicit-any': 'off',
      '@typescript-eslint/no-unsafe-function-type': 'off',
    },
  },
  {
    ignores: ['src-tauri/', 'dist/', 'node_modules/'],
  },
)
