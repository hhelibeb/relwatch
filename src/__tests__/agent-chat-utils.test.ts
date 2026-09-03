import { describe, it, expect } from 'vitest'
import type { AgentChatMessage, AgentRunSummary } from '../api/agent'
import { InvokeI18nError } from '../api/client'
import {
  bashExitLabel,
  canRetry,
  errorKey,
  escapeRegExp,
  formatCostUsd,
  isToolError,
  runDurationText,
  runEntities,
  runErrorText,
  runFiles,
  runModel,
  runModelLabel,
  splitUserBlocks,
  timeAdjacent,
  toolCardBody,
  toolCardName,
} from '../components/agent/agentChatUtils'

/** t 替身：键 + 参数简单拼接，不依赖 i18n 全局 locale 状态（保持纯函数单测）。 */
const stubT = (key: string, ...args: string[]): string => (args.length > 0 ? `${key}(${args.join(',')})` : key)

function makeRun(over: Partial<AgentRunSummary> = {}): AgentRunSummary {
  return {
    id: 1,
    session_key: 'k',
    skill_path: null,
    entities: '[]',
    instruction: 'do',
    model: null,
    session_path: null,
    status: 'success',
    exit_code: 0,
    error: null,
    started_at: null,
    finished_at: null,
    created_at: '2025-01-01T00:00:00.000Z',
    files: null,
    ...over,
  } as AgentRunSummary
}

function userMsg(text: string): AgentChatMessage {
  return { role: 'user', timestamp: '2025-01-01T00:00:00.000Z', model: null, blocks: [{ kind: 'text', text }] }
}

describe('splitUserBlocks', () => {
  it('skill 块剥离：<skill> 全文折叠为空，仅保留后续指令', () => {
    const long = '# 全文很长\n' + 'x'.repeat(2000)
    const msg = userMsg(`<skill name="yt" location="E:\\s\\SKILL.md">\n${long}\n</skill>\n执行 请开始。`)
    const r = splitUserBlocks(msg.blocks)
    expect(r.main).toBe('执行 请开始。')
    expect(r.folded).toBeNull()
    expect(r.main).not.toContain('全文很长')
  })

  it('有标签：<用户指令> 在中间时主文本取标签内容，脚手架折叠为详情', () => {
    const msg = userMsg('以下是订阅信息……\n<用户指令>\n中间指令\n</用户指令>\n以上是指令声明')
    const r = splitUserBlocks(msg.blocks)
    expect(r.main).toBe('中间指令')
    expect(r.folded).toBe('以下是订阅信息……\n\n以上是指令声明')
  })

  it('旧格式（无标签）：整段作为主文本，folded 为 null', () => {
    const msg = userMsg('不用，有没有提到努比亚')
    const r = splitUserBlocks(msg.blocks)
    expect(r.main).toBe('不用，有没有提到努比亚')
    expect(r.folded).toBeNull()
  })

  it('整条被 <用户指令> 包裹：标签剥离，仅内容可见', () => {
    const msg = userMsg('<用户指令>\n内容\n</用户指令>')
    const r = splitUserBlocks(msg.blocks)
    expect(r.main).toBe('内容')
    expect(r.folded).toBeNull()
  })
})

describe('run 记录解析（坏 JSON 容错）', () => {
  it('runEntities：坏 JSON / 缺 run 返回空数组', () => {
    expect(runEntities(undefined)).toEqual([])
    expect(runEntities(makeRun({ entities: 'not-json' }))).toEqual([])
    expect(runEntities(makeRun({ entities: '[{"kind":"release","id":7}]' }))).toEqual([{ kind: 'release', id: 7 }])
  })

  it('runFiles：坏 JSON / 非字符串数组过滤 / 缺 files 返回空数组', () => {
    expect(runFiles(makeRun({ files: null }))).toEqual([])
    expect(runFiles(makeRun({ files: 'not-json' }))).toEqual([])
    expect(runFiles(makeRun({ files: '["a.log", 42, "b.log"]' } as unknown as Partial<AgentRunSummary>))).toEqual(['a.log', 'b.log'])
    expect(runFiles(makeRun({ files: '{"x":1}' } as unknown as Partial<AgentRunSummary>))).toEqual([])
  })

  it('runModel：坏 JSON / 空 model 返回 null', () => {
    expect(runModel(makeRun({ model: null }))).toBeNull()
    expect(runModel(makeRun({ model: 'not-json' }))).toBeNull()
    expect(runModel(makeRun({ model: '{"provider":"anthropic","model_id":"claude-test"}' }))).toEqual({
      provider: 'anthropic',
      model_id: 'claude-test',
    })
  })

  it('runModelLabel：有模型显示 model_id，否则走 i18n「默认」', () => {
    expect(runModelLabel(makeRun({ model: '{"provider":"p","model_id":"m1"}' }), stubT)).toBe('m1')
    expect(runModelLabel(makeRun(), stubT)).toBe('agent.run_model_default')
  })

  it('runEntities：合法 JSON 直接反序列化', () => {
    expect(runEntities(makeRun({ entities: '[{"kind":"source","id":3}]' }))).toEqual([{ kind: 'source', id: 3 }])
  })
})

describe('formatCostUsd 三个量级档', () => {
  it('>= 1 美元：2 位小数', () => {
    expect(formatCostUsd(2_500_000)).toBe('2.50')
  })
  it('>= 0.01 美元：4 位小数', () => {
    expect(formatCostUsd(41_244)).toBe('0.0412')
  })
  it('< 0.01 美元：6 位小数（微额不为零）', () => {
    expect(formatCostUsd(123)).toBe('0.000123')
  })
})

describe('runErrorText / runDurationText（t 注入）', () => {
  it('runErrorText：err.agent.timeout|300 拆键传参', () => {
    expect(runErrorText(makeRun({ error: 'err.agent.timeout|300' }), stubT)).toBe('err.agent.timeout(300)')
  })

  it('runErrorText：无 error / i18n 未命中返回 null（不渲染裸键）', () => {
    expect(runErrorText(undefined, stubT)).toBeNull()
    expect(runErrorText(makeRun({ error: null }), stubT)).toBeNull()
    const identity = (key: string) => key
    expect(runErrorText(makeRun({ error: 'err.agent.timeout|300' }), identity)).toBeNull()
  })

  it('runDurationText：秒 / 分+秒 / 分 / 时+分 四档', () => {
    const base = '2025-01-01T00:00:00.000Z'
    expect(runDurationText(makeRun(), stubT)).toBe('—')
    expect(runDurationText(makeRun({ started_at: base, finished_at: '2025-01-01T00:00:05.000Z' }), stubT)).toBe(
      'agent.duration_secs(5)',
    )
    expect(runDurationText(makeRun({ started_at: base, finished_at: '2025-01-01T00:01:05.000Z' }), stubT)).toBe(
      'agent.duration_min_secs(1,5)',
    )
    expect(runDurationText(makeRun({ started_at: base, finished_at: '2025-01-01T00:02:00.000Z' }), stubT)).toBe(
      'agent.duration_min(2)',
    )
    expect(runDurationText(makeRun({ started_at: base, finished_at: '2025-01-01T01:02:00.000Z' }), stubT)).toBe(
      'agent.duration_hour_min(1,2)',
    )
  })
})

describe('canRetry / timeAdjacent', () => {
  it('canRetry：failed/timeout/cancelled/unknown 可重试，success/pending/running/undefined 不可', () => {
    for (const status of ['failed', 'timeout', 'cancelled', 'unknown']) {
      expect(canRetry(makeRun({ status }))).toBe(true)
    }
    for (const status of ['success', 'pending', 'running']) {
      expect(canRetry(makeRun({ status }))).toBe(false)
    }
    expect(canRetry(undefined)).toBe(false)
  })

  it('timeAdjacent：60 秒窗内为真，坏时间为假', () => {
    expect(timeAdjacent('2025-01-01T00:00:00.000Z', '2025-01-01T00:00:59.000Z')).toBe(true)
    expect(timeAdjacent('2025-01-01T00:00:00.000Z', '2025-01-01T00:01:01.000Z')).toBe(false)
    expect(timeAdjacent('not-a-date', '2025-01-01T00:00:00.000Z')).toBe(false)
  })
})

describe('工具卡片 / 消息块辅助', () => {
  const toolResultMsg: AgentChatMessage = {
    role: 'toolResult',
    timestamp: '2025-01-01T00:00:00.000Z',
    model: null,
    blocks: [{ kind: 'toolResult', id: 'c1', tool_name: 'bash', text: 'src\n', is_error: false }],
  }
  const bashMsg: AgentChatMessage = {
    role: 'bash',
    timestamp: '2025-01-01T00:00:00.000Z',
    model: null,
    blocks: [{ kind: 'bash', command: 'ls', output: 'a.txt', exit_code: 2, truncated: false }],
  }

  it('isToolError：is_error 块判定', () => {
    expect(isToolError(toolResultMsg)).toBe(false)
    const errMsg = {
      ...toolResultMsg,
      blocks: [{ ...(toolResultMsg.blocks[0] as { kind: 'toolResult'; is_error: boolean }), is_error: true }],
    } as AgentChatMessage
    expect(isToolError(errMsg)).toBe(true)
  })

  it('toolCardName：toolResult 取 tool_name，bash 固定 bash，其他回退 role', () => {
    expect(toolCardName(toolResultMsg)).toBe('bash')
    expect(toolCardName(bashMsg)).toBe('bash')
    expect(toolCardName(userMsg('hi'))).toBe('user')
  })

  it('bashExitLabel：exit N（exit_code null 显示 ?），非 bash 返回空串', () => {
    expect(bashExitLabel(bashMsg)).toBe('exit 2')
    const nullCode = {
      ...bashMsg,
      blocks: [{ ...(bashMsg.blocks[0] as { kind: 'bash'; exit_code: number | null }), exit_code: null }],
    } as AgentChatMessage
    expect(bashExitLabel(nullCode)).toBe('exit ?')
    expect(bashExitLabel(toolResultMsg)).toBe('')
  })

  it('toolCardBody：toolResult 正文 / bash 命令+输出，空行过滤', () => {
    expect(toolCardBody(toolResultMsg)).toBe('src\n')
    expect(toolCardBody(bashMsg)).toBe('$ ls\na.txt')
  })

  it('escapeRegExp：正则元字符转义', () => {
    const re = new RegExp(`@${escapeRegExp('code-review.v2')}\\s*`)
    expect(re.test('@code-review.v2 ')).toBe(true)
    expect(re.test('@code-reviewXv2 ')).toBe(false)
  })
})

describe('errorKey', () => {
  it('InvokeI18nError 直接带 key', () => {
    const e = new InvokeI18nError('err.agent.export_cancelled', [], '已取消')
    expect(errorKey(e)).toBe('err.agent.export_cancelled')
  })

  it('普通错误仅当原文是 err.* 键时返回（含 Error: 前缀剥离与 | 参数截断）', () => {
    expect(errorKey(new Error('err.agent.timeout|300'))).toBe('err.agent.timeout')
    expect(errorKey('err.agent.foo')).toBe('err.agent.foo')
    expect(errorKey(new Error('boom'))).toBeNull()
    expect(errorKey('boom')).toBeNull()
  })
})
