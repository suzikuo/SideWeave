//! 用户插件沙箱桥接：注入到插件 HTML 顶部的 SDK。
//! 插件通过 window.agileTiles 与宿主通信（postMessage → 详情页 → Tauri 命令），
//! 沙箱 iframe 无 Tauri 权限，只能使用这些受限能力。

pub const BRIDGE_JS: &str = r#"(function () {
  // The postMessage bridge is only for sandboxed plugin iframes. A plugin
  // opened in its own Tauri window must use the native invoke fallback; when
  // parent === self, posting to parent loops back into this listener and can
  // resolve an RPC with an undefined result.
  if (window.parent === window || window.__agileTilesBridge) return;
  window.__agileTilesBridge = true;
  var seq = 0;
  var pending = {};
  var eventListeners = {};
  var panelFillCaptured = false;
  var panelFill = 'transparent';
  var panelFillTarget = null;
  function cloneMessage(value) {
    try {
      var serialized = JSON.stringify(value);
      return serialized === undefined ? null : JSON.parse(serialized);
    } catch (error) {
      throw new Error('agileTiles: payload is not serializable');
    }
  }
  function rpc(action, payload) {
    return new Promise(function (resolve, reject) {
      var id = ++seq;
      var timer = setTimeout(function () {
        if (pending[id]) {
          delete pending[id];
          reject(new Error('agileTiles: timeout'));
        }
      }, 8000);
      pending[id] = {
        resolve: function (v) { clearTimeout(timer); resolve(v); },
        reject: function (e) { clearTimeout(timer); reject(e); }
      };
      window.parent.postMessage(cloneMessage({
        __agileTiles: true,
        id: id,
        plugin_id: window.__agileTilesPluginId || '',
        action: action,
        payload: payload
      }), '*');
    });
  }
  function clampOpacity(value) {
    var opacity = Number(value);
    if (!isFinite(opacity)) return 1;
    return Math.max(0, Math.min(1, opacity));
  }
  function isTransparentColor(value) {
    return !value || value === 'transparent' ||
      /^rgba\([^,]+,[^,]+,[^,]+,\s*0(?:\.0+)?\s*\)$/i.test(value);
  }
  function applyHostTheme(theme) {
    var root = document.documentElement;
    if (!root) return;
    var variables = theme && theme.variables || {};
    var panelOpacity = clampOpacity(variables['--detail-panel-opacity']);
    var componentOpacity = clampOpacity(variables['--component-opacity']);
    var imageOpacity = clampOpacity(variables['--detail-image-opacity']);
    root.style.setProperty('--detail-panel-opacity', String(panelOpacity));
    root.style.setProperty('--component-opacity', String(componentOpacity));
    root.style.setProperty('--detail-image-opacity', String(imageOpacity));

    var body = document.body;
    if (!body) {
      document.addEventListener('DOMContentLoaded', function () { applyHostTheme(theme); }, { once: true });
      return;
    }

    if (!panelFillCaptured) {
      var bodyFill = getComputedStyle(body).backgroundColor;
      var rootFill = getComputedStyle(root).backgroundColor;
      panelFill = !isTransparentColor(bodyFill) ? bodyFill :
        (!isTransparentColor(rootFill) ? rootFill : 'transparent');
      panelFillTarget = body;

      // Many plugin pages put their solid page background on a full-window
      // app root instead of body. Treat that as the page fill too, while
      // leaving smaller cards and component surfaces untouched.
      var largestArea = 0;
      Array.prototype.forEach.call(body.children, function (child) {
        var rect = child.getBoundingClientRect();
        var coversViewport = rect.width >= window.innerWidth * 0.85 &&
          rect.height >= window.innerHeight * 0.85;
        var area = rect.width * rect.height;
        var childFill = getComputedStyle(child).backgroundColor;
        if (coversViewport && area > largestArea && !isTransparentColor(childFill)) {
          largestArea = area;
          panelFillTarget = child;
          panelFill = childFill;
        }
      });
      root.style.setProperty('--agile-tiles-plugin-panel-fill', panelFill);
      panelFillCaptured = true;
    }

    // Keep plugin page artwork and all foreground content intact. Only the
    // page-level background color is blended with the host surface.
    root.style.setProperty('background-color', 'transparent', 'important');
    var target = panelFillTarget && body.contains(panelFillTarget) ? panelFillTarget : body;
    if (target !== body) {
      body.style.setProperty('background-color', 'transparent', 'important');
    }
    target.style.setProperty(
      'background-color',
      'color-mix(in srgb, var(--agile-tiles-plugin-panel-fill, transparent) calc(var(--detail-panel-opacity, 1) * 100%), transparent)',
      'important'
    );
  }
  window.addEventListener('message', function (e) {
    var d = e.data;
    if (e.source !== window.parent ||
        !d || d.__agileTiles !== true ||
        d.plugin_id !== (window.__agileTilesPluginId || '')) return;
    if (d.type === 'theme') {
      applyHostTheme(d.theme);
      return;
    }
    if (d.type === 'event' && d.event && d.event.topic) {
      var listeners = eventListeners[d.event.topic] || [];
      listeners.slice().forEach(function (listener) { listener(d.event.payload); });
      return;
    }
    if (!d.id || !pending[d.id]) return;
    if (d && d.__agileTiles === true && d.id && pending[d.id]) {
      var p = pending[d.id];
      delete pending[d.id];
      if (d.error) p.reject(new Error(d.error));
      else p.resolve(d.result);
    }
  });
  var api = {
    apiVersion: 1,
    plugin: {
      call: function (method, payload, timeoutMs) {
        return rpc('call', { method: method, payload: payload, timeout_ms: timeoutMs });
      },
      onEvent: function (topic, handler) {
        if (!eventListeners[topic]) eventListeners[topic] = [];
        eventListeners[topic].push(handler);
        return function () {
          eventListeners[topic] = (eventListeners[topic] || []).filter(function (item) { return item !== handler; });
        };
      }
    },
    storage: {
      /** 读取本插件命名空间下的存储值（字符串） */
      get: function (key) { return rpc('call', { method: 'host.storage.get', payload: { key: key } }); },
      /** 写入存储值（≤256KB 字符串） */
      set: function (key, value) { return rpc('call', { method: 'host.storage.set', payload: { key: key, value: value } }); },
      remove: function (key) { return rpc('call', { method: 'host.storage.remove', payload: { key: key } }); }
    },
    /** 用系统默认浏览器打开链接 */
    openExternal: function (url) { return rpc('call', { method: 'host.system.open_external', payload: { url: url } }); }
  };
  window.agileTiles = api;
})();
"#;
