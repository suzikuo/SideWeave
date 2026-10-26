import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { setImmediate } from 'node:timers/promises'
import test from 'node:test'
import { runInNewContext } from 'node:vm'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'

// Render the real component with Vue's renderer. Track existing iframe moves:
// native DOM insertion of an attached iframe destroys/reloads its document.
const filename = new URL('../../src/views/UserPluginHostView.vue', import.meta.url)
const { descriptor } = parse(readFileSync(filename, 'utf8'))
const script = compileScript(descriptor, { id: 'detail-host-test', inlineTemplate: true })
const { code } = transformSync(script.content, { loader: 'ts', format: 'cjs' })
const require = createRequire(import.meta.url)

class Node {
  constructor(tag) {
    this.tag = tag
    this.parent = null
    this.children = []
    this.props = {}
    this.moves = 0
  }
}

class Frame extends Node {
  constructor() {
    super('iframe')
    this.messages = []
    this.contentWindow = { postMessage: (message) => this.messages.push(message) }
  }
}

async function flush() {
  await setImmediate()
  await vue.nextTick()
}

async function mountHost(t, initialProps = {}) {
  const calls = []
  const closed = []
  const listeners = new Map()
  const renderer = vue.createRenderer({
    createElement: (tag) => tag === 'iframe' ? new Frame() : new Node(tag),
    createText: () => new Node('#text'),
    createComment: () => new Node('#comment'),
    setText: () => {},
    setElementText: () => {},
    parentNode: (node) => node.parent,
    nextSibling: (node) => node.parent?.children[node.parent.children.indexOf(node) + 1] ?? null,
    patchProp: (node, key, _old, value) => { node.props[key] = value },
    insert(node, parent, anchor = null) {
      if (node.parent) {
        node.moves += 1
        node.parent.children.splice(node.parent.children.indexOf(node), 1)
      }
      node.parent = parent
      const index = anchor ? parent.children.indexOf(anchor) : -1
      parent.children.splice(index < 0 ? parent.children.length : index, 0, node)
    },
    remove(node) {
      node.parent.children.splice(node.parent.children.indexOf(node), 1)
      node.parent = null
    }
  })
  const registry = ['bookmarks', 'notes', 'time', 'clipboard', 'toolbox'].map((id) => ({
    manifest: { id, ui: { entry: 'ui/index.html' } },
    runtime: 'dynamic_library', source: 'installed-dylib', effectiveTrust: 'trusted_local'
  }))
  const settingsRef = vue.ref(null)
  const mocks = {
    vue,
    '@tauri-apps/api/core': { invoke: async () => {} },
    '@tauri-apps/api/event': { listen: async () => () => {} },
    '../ui/Icon.vue': { render: () => null },
    '../lib/api': { closeDetail: async () => { closed.push(true) } },
    '../lib/pluginApi': {
      pluginCall: async (...args) => { calls.push(args); return { items: ['saved data'] } }
    },
    '../lib/marketApi': { pluginRegistryList: async () => registry },
    '../lib/store': { settingsRef }
  }
  const module = { exports: {} }
  runInNewContext(code, {
    module, exports: module.exports,
    require: (name) => mocks[name] ?? require(name),
    HTMLIFrameElement: Frame,
    window: {
      setTimeout, clearTimeout,
      addEventListener: (name, handler) => listeners.set(name, handler),
      removeEventListener: (name) => listeners.delete(name)
    },
    document: { documentElement: { dataset: {} } },
    getComputedStyle: () => ({ getPropertyValue: () => '', colorScheme: 'dark' }),
    MutationObserver: class { observe() {} disconnect() {} },
    TextEncoder
  })
  const props = vue.reactive({ pluginId: 'bookmarks', active: true, ...initialProps })
  const root = new Node('root')
  const app = renderer.createApp({ render: () => vue.h(module.exports.default, props) })
  app.mount(root)
  t.after(() => app.unmount())
  await flush()

  function frames(node = root) {
    return node.children.flatMap((child) => child instanceof Frame ? [child] : frames(child))
  }
  function frame(id) {
    return frames().find((item) => item.props.title === id)
  }
  async function switchTo(id) {
    props.pluginId = id
    await flush()
  }
  async function load(id) {
    frame(id).props.onLoad()
    await flush()
  }
  async function send(source, pluginId, extra = {}) {
    await listeners.get('message')({ source, data: {
      __agileTiles: true, id: 1, plugin_id: pluginId,
      action: 'call', payload: { method: 'list' }, ...extra
    } })
  }
  return { props, calls, closed, frames, frame, switchTo, load, send }
}

test('switching repeatedly preserves cached iframe documents and their state', async (t) => {
  const host = await mountHost(t)
  await host.load('bookmarks')
  const bookmarks = host.frame('bookmarks')
  bookmarks.savedState = { search: 'my query', scrollTop: 42 }
  await host.switchTo('notes')
  await host.load('notes')
  const notes = host.frame('notes')
  for (let i = 0; i < 5; i += 1) {
    await host.switchTo('bookmarks')
    await host.switchTo('notes')
  }
  assert.equal(host.frame('bookmarks'), bookmarks)
  assert.equal(host.frame('notes'), notes)
  assert.equal(bookmarks.moves, 0, 'moving a cached iframe reloads its document')
  assert.equal(notes.moves, 0)
  assert.deepEqual(bookmarks.savedState, { search: 'my query', scrollTop: 42 })
  assert.equal(bookmarks.messages.filter((m) => m.type === 'lifecycle').at(-1).active, false)
  assert.equal(notes.messages.filter((m) => m.type === 'lifecycle').at(-1).active, true)
})

test('evicts the least recently used inactive page without moving survivors', async (t) => {
  const host = await mountHost(t)
  for (const id of ['notes', 'time', 'clipboard']) await host.switchTo(id)
  const bookmarks = host.frame('bookmarks')
  await host.switchTo('bookmarks')
  await host.switchTo('toolbox')
  assert.equal(host.frames().length, 4)
  assert.equal(host.frame('notes'), undefined)
  assert.equal(host.frame('bookmarks'), bookmarks)
  assert.ok(host.frame('toolbox'))
  for (const frame of host.frames()) assert.equal(frame.moves, 0)
})

test('initialization RPC completes while the detail surface is hidden', async (t) => {
  const host = await mountHost(t, { active: false })
  const frame = host.frame('bookmarks')
  await host.load('bookmarks')
  await host.send(frame.contentWindow, 'bookmarks')
  assert.equal(host.calls.length, 1)
  assert.equal(frame.messages.at(-1).result.items[0], 'saved data')
  host.props.active = true
  await flush()
  assert.equal(host.frame('bookmarks'), frame)
})

test('async continuation RPC completes after switching away from its page', async (t) => {
  const host = await mountHost(t)
  await host.load('bookmarks')
  const frame = host.frame('bookmarks')
  await host.switchTo('notes')
  await host.send(frame.contentWindow, 'bookmarks')
  assert.equal(host.calls[0][0], 'bookmarks')
  assert.equal(frame.messages.at(-1).result.items[0], 'saved data')
  await host.send(frame.contentWindow, 'bookmarks', { type: 'close-detail' })
  assert.equal(host.closed.length, 0, 'inactive frames cannot close the current page')
})

test('rejects unregistered windows, spoofed plugin IDs, and evicted frames', async (t) => {
  const host = await mountHost(t)
  const frame = host.frame('bookmarks')
  await host.send({}, 'bookmarks')
  await host.send(frame.contentWindow, 'notes')
  assert.equal(host.calls.length, 0)
  for (const id of ['notes', 'time', 'clipboard', 'toolbox']) await host.switchTo(id)
  await host.send(frame.contentWindow, 'bookmarks')
  assert.equal(host.calls.length, 0)
  await host.send(host.frame('toolbox').contentWindow, 'toolbox', { type: 'close-detail' })
  assert.equal(host.closed.length, 1)
})
