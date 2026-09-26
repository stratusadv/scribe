import js from '@eslint/js'
import { defineConfig, globalIgnores } from 'eslint/config'
import globals from 'globals'
import pluginVue from 'eslint-plugin-vue'
import tseslint from 'typescript-eslint'

export default defineConfig(
    globalIgnores([
        'design/**',
        'dist/**',
        'node_modules/**',
        'public/**',
        'src-tauri/**',
    ]),
    {
        files: ['**/*.{ts,vue}'],
        extends: [
            js.configs.recommended,
            tseslint.configs.strictTypeChecked,
            tseslint.configs.stylisticTypeChecked,
            pluginVue.configs['flat/recommended'],
        ],
        languageOptions: {
            ecmaVersion: 2022,
            globals: globals.browser,
            parserOptions: {
                extraFileExtensions: ['.vue'],
                parser: tseslint.parser,
                projectService: true,
                tsconfigRootDir: import.meta.dirname,
            },
            sourceType: 'module',
        },
        rules: {
            'no-undef': 'off',

            'vue/html-indent': ['error', 4],
            'vue/max-attributes-per-line': [
                'error',
                {
                    singleline: { max: Number.POSITIVE_INFINITY },
                    multiline: { max: 1 },
                },
            ],
            'vue/singleline-html-element-content-newline': 'off',
            'vue/html-self-closing': [
                'error',
                {
                    html: { void: 'always', normal: 'always', component: 'always' },
                    svg: 'always',
                    math: 'always',
                },
            ],
            'vue/prop-name-casing': 'off',

            'vue/max-len': [
                'error',
                {
                    code: 100,
                    template: 100,
                    ignoreComments: false,
                    ignoreHTMLAttributeValues: true,
                    ignoreHTMLTextContents: true,
                    ignoreRegExpLiterals: true,
                    ignoreStrings: true,
                    ignoreTemplateLiterals: true,
                    ignoreUrls: true,
                },
            ],

            '@typescript-eslint/restrict-template-expressions': ['error', { allowNumber: true }],
            '@typescript-eslint/no-invalid-void-type': [
                'error',
                { allowInGenericTypeArguments: true },
            ],
        },
    },
    {
        files: ['vite.config.ts'],
        languageOptions: {
            globals: globals.node,
        },
    },
)
