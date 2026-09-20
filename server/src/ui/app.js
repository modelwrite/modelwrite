// SPDX-License-Identifier: AGPL-3.0-or-later
//
// modelwrite workbench enhancement. No build step, no dependency, nothing fetched: this file
// is served by the same router as the page it enhances, and every value it reads comes from
// data-* attributes the server-rendered page already carries. It renders only with
// textContent / createElement - never innerHTML on model data - so the enhancement can never
// be a second, weaker source of truth, and a page with JavaScript disabled is byte-for-byte
// the server-rendered list it always was.
(function () {
  'use strict';

  function ready(fn) {
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', fn);
    } else {
      fn();
    }
  }

  ready(function () {
    if (document.querySelector('.model-section')) {
      enhanceModelPage();
    } else if (document.querySelector('svg g.node')) {
      enhanceDiagramPage();
    }
  });

  // ---- small helpers -------------------------------------------------------

  function each(list, fn) {
    for (var i = 0; i < list.length; i += 1) {
      fn(list[i], i);
    }
  }

  // Create a DOM element; model data reaches the page only through textContent.
  function el(tag, className, text) {
    var node = document.createElement(tag);
    if (className) {
      node.className = className;
    }
    if (text !== undefined && text !== null) {
      node.textContent = text;
    }
    return node;
  }

  function readAttr(element, name) {
    var raw = element.getAttribute('data-' + name);
    return raw === null ? '' : raw;
  }

  function readJson(element, name) {
    var raw = element.getAttribute('data-' + name);
    if (!raw) {
      return undefined;
    }
    try {
      return JSON.parse(raw);
    } catch (error) {
      return undefined;
    }
  }

  function childUl(li) {
    for (var i = 0; i < li.children.length; i += 1) {
      if (li.children[i].tagName === 'UL') {
        return li.children[i];
      }
    }
    return null;
  }

  function readSelectFromUrl() {
    if (typeof URLSearchParams === 'undefined') {
      return null;
    }
    return new URLSearchParams(window.location.search).get('select');
  }

  function setSelectParam(key) {
    if (typeof URL === 'undefined' || typeof window.history === 'undefined') {
      return;
    }
    var url = new URL(window.location.href);
    if (key) {
      url.searchParams.set('select', key);
    } else {
      url.searchParams.delete('select');
    }
    window.history.replaceState(null, '', url.toString());
  }

  // ---- diagram page: pan / zoom / select / filter / search -----------------

  function enhanceDiagramPage() {
    var svg = document.querySelector('svg.mw-diagram-svg');
    if (!svg) {
      return;
    }
    var viewport = svg.parentElement;
    var nodeEls = svg.querySelectorAll('g.node');
    var edgeEls = svg.querySelectorAll('g.edge');

    // The camera is the viewBox: an x y w h window in SVG user units. The server renders the
    // full diagram (fit-to-view) and the enhancement pans/zooms by moving that window, so the
    // page is still a complete, laid-out picture with JavaScript disabled.
    var fullW = 1;
    var fullH = 1;
    if (svg.viewBox && svg.viewBox.baseVal) {
      fullW = svg.viewBox.baseVal.width || 1;
      fullH = svg.viewBox.baseVal.height || 1;
    }
    var vb = { x: 0, y: 0, w: fullW, h: fullH };

    function applyViewBox() {
      svg.setAttribute('viewBox', vb.x + ' ' + vb.y + ' ' + vb.w + ' ' + vb.h);
    }

    function fitToView() {
      vb.x = 0;
      vb.y = 0;
      vb.w = fullW;
      vb.h = fullH;
      applyViewBox();
    }

    function svgPoint(clientX, clientY) {
      var pt = svg.createSVGPoint();
      pt.x = clientX;
      pt.y = clientY;
      var ctm = svg.getScreenCTM();
      if (!ctm) {
        return null;
      }
      return pt.matrixTransform(ctm.inverse());
    }

    function clamp(value, min, max) {
      return Math.min(Math.max(value, min), max);
    }

    function zoomAt(clientX, clientY, factor) {
      var p = svgPoint(clientX, clientY);
      var fx = p ? (p.x - vb.x) / vb.w : 0.5;
      var fy = p ? (p.y - vb.y) / vb.h : 0.5;
      var nw = clamp(vb.w * factor, fullW / 40, fullW * 40);
      var nh = clamp(vb.h * factor, fullH / 40, fullH * 40);
      vb.x = (p ? p.x : vb.x + fx * vb.w) - fx * nw;
      vb.y = (p ? p.y : vb.y + fy * vb.h) - fy * nh;
      vb.w = nw;
      vb.h = nh;
      applyViewBox();
    }

    function zoomBy(factor) {
      var rect = viewport.getBoundingClientRect();
      zoomAt(rect.left + rect.width / 2, rect.top + rect.height / 2, factor);
    }

    function panBy(dxPx, dyPx) {
      var scale = vb.w / (viewport.clientWidth || 1);
      vb.x -= dxPx * scale;
      vb.y -= dyPx * scale;
      applyViewBox();
    }

    function centerOn(nodeEl) {
      var rect = nodeEl.querySelector('rect');
      if (!rect) {
        return;
      }
      var cx = parseFloat(rect.getAttribute('x')) + parseFloat(rect.getAttribute('width')) / 2;
      var cy = parseFloat(rect.getAttribute('y')) + parseFloat(rect.getAttribute('height')) / 2;
      vb.x = clamp(cx - vb.w / 2, 0, Math.max(0, fullW - vb.w));
      vb.y = clamp(cy - vb.h / 2, 0, Math.max(0, fullH - vb.h));
      applyViewBox();
    }

    // The selection card, overlaid in the viewport corner, mirrors the model page's
    // properties pane for the node under the cursor/click.
    var props = el('aside', 'diagram-props');
    var propsTitle = el('h3', 'mw-props-title', 'Selection');
    var propsList = el('dl', 'mw-props-list');
    props.appendChild(propsTitle);
    props.appendChild(propsList);
    viewport.appendChild(props);

    function incidentEdges(nodeEl) {
      var id = nodeEl.getAttribute('data-mw-id');
      var out = [];
      each(edgeEls, function (edge) {
        if (
          edge.getAttribute('data-mw-source') === id ||
          edge.getAttribute('data-mw-target') === id
        ) {
          out.push(edge);
        }
      });
      return out;
    }

    function markIncident(nodeEl, on) {
      each(incidentEdges(nodeEl), function (edge) {
        edge.classList.toggle('mw-incident', on);
      });
    }

    function selectNode(nodeEl) {
      each(nodeEls, function (n) {
        n.classList.remove('mw-selected-node');
      });
      each(edgeEls, function (edge) {
        edge.classList.remove('mw-incident');
      });
      nodeEl.classList.add('mw-selected-node');
      markIncident(nodeEl, true);
      setSelectParam(nodeEl.getAttribute('data-mw-id'));

      var name = nodeEl.getAttribute('data-mw-name') || nodeEl.getAttribute('data-mw-id');
      var kind = nodeEl.getAttribute('data-mw-kind') || '';
      propsTitle.textContent = name;
      propsList.textContent = '';
      appendRow(propsList, 'id', nodeEl.getAttribute('data-mw-id'));
      appendRow(propsList, 'name', name);
      appendRow(propsList, 'kind', kind);
    }

    function neighbourIds(nodeEl) {
      var id = nodeEl.getAttribute('data-mw-id');
      var set = {};
      each(edgeEls, function (edge) {
        if (edge.getAttribute('data-mw-source') === id) {
          set[edge.getAttribute('data-mw-target')] = true;
        } else if (edge.getAttribute('data-mw-target') === id) {
          set[edge.getAttribute('data-mw-source')] = true;
        }
      });
      return set;
    }

    function highlightNeighbours(nodeEl) {
      var id = nodeEl.getAttribute('data-mw-id');
      var others = neighbourIds(nodeEl);
      svg.classList.add('dimmed');
      each(nodeEls, function (n) {
        var nid = n.getAttribute('data-mw-id');
        if (nid === id || others[nid]) {
          n.classList.add('mw-active');
        }
      });
      markIncident(nodeEl, true);
    }

    function clearHighlight() {
      svg.classList.remove('dimmed');
      each(nodeEls, function (n) {
        n.classList.remove('mw-active');
      });
      each(edgeEls, function (edge) {
        edge.classList.remove('mw-incident');
      });
    }

    // ---- node wiring: click selects, hover highlights the neighbourhood ----
    each(nodeEls, function (nodeEl) {
      nodeEl.addEventListener('mouseenter', function () {
        nodeEl.classList.add('mw-hover');
        highlightNeighbours(nodeEl);
      });
      nodeEl.addEventListener('mouseleave', function () {
        nodeEl.classList.remove('mw-hover');
        clearHighlight();
      });
      nodeEl.addEventListener('click', function (event) {
        event.stopPropagation();
        selectNode(nodeEl);
      });
    });

    // ---- kind filter chips ----
    var activeKinds = {};
    each(document.querySelectorAll('.kind-filter'), function (chip) {
      chip.addEventListener('click', function () {
        var kind = chip.getAttribute('data-mw-kind');
        if (activeKinds[kind]) {
          delete activeKinds[kind];
          chip.classList.remove('active');
        } else {
          activeKinds[kind] = true;
          chip.classList.add('active');
        }
        var hasActive = Object.keys(activeKinds).length > 0;
        each(nodeEls, function (n) {
          var hide = hasActive && !activeKinds[n.getAttribute('data-mw-kind')];
          n.classList.toggle('mw-filtered-out', hide);
        });
      });
    });

    // ---- search highlight ----
    var search = document.querySelector('.mw-diagram-search');
    if (search) {
      search.addEventListener('input', function () {
        var q = search.value.trim().toLowerCase();
        each(nodeEls, function (n) {
          var hay = (
            (n.getAttribute('data-mw-name') || '') + ' ' +
            (n.getAttribute('data-mw-id') || '') + ' ' +
            (n.getAttribute('data-mw-kind') || '')
          ).toLowerCase();
          var match = q !== '' && hay.indexOf(q) !== -1;
          n.classList.toggle('mw-search-match', match);
        });
      });
    }

    // ---- zoom / fit buttons ----
    var zoomIn = document.querySelector('.mw-zoom-in');
    var zoomOut = document.querySelector('.mw-zoom-out');
    var fit = document.querySelector('.mw-fit');
    if (zoomIn) {
      zoomIn.addEventListener('click', function () { zoomBy(1.3); });
    }
    if (zoomOut) {
      zoomOut.addEventListener('click', function () { zoomBy(1 / 1.3); });
    }
    if (fit) {
      fit.addEventListener('click', fitToView);
    }

    // ---- wheel zoom around the cursor ----
    viewport.addEventListener(
      'wheel',
      function (event) {
        event.preventDefault();
        var factor = event.deltaY < 0 ? 1.12 : 1 / 1.12;
        zoomAt(event.clientX, event.clientY, factor);
      },
      { passive: false }
    );

    // ---- drag to pan (on the background, not on nodes/edges) ----
    var dragging = false;
    var lastX = 0;
    var lastY = 0;
    viewport.addEventListener('mousedown', function (event) {
      var target = event.target;
      if (target.closest && (target.closest('g.node') || target.closest('g.edge'))) {
        return;
      }
      dragging = true;
      lastX = event.clientX;
      lastY = event.clientY;
      svg.classList.add('dragging');
      event.preventDefault();
    });
    window.addEventListener('mousemove', function (event) {
      if (!dragging) {
        return;
      }
      panBy(event.clientX - lastX, event.clientY - lastY);
      lastX = event.clientX;
      lastY = event.clientY;
    });
    window.addEventListener('mouseup', function () {
      if (dragging) {
        dragging = false;
        svg.classList.remove('dragging');
      }
    });

    // ---- honour a ?select= deep link (the page already opens fit-to-view) ----
    var key = readSelectFromUrl();
    if (key) {
      each(nodeEls, function (nodeEl) {
        if (nodeEl.getAttribute('data-mw-id') === key) {
          selectNode(nodeEl);
          centerOn(nodeEl);
        }
      });
    }
  }


  // ---- model page: containment tree | content | properties -----------------

  function enhanceModelPage() {
    var main = document.querySelector('main');
    if (!main) {
      return;
    }
    main.classList.add('mw-model-ide');

    var workbench = el('div', 'mw-workbench');
    var treePanel = el('aside', 'mw-tree-panel');
    var content = el('div', 'mw-content');
    var propsPanel = el('aside', 'mw-props-panel');

    // Panel chrome: a header on each side pane so the layout reads as three
    // deliberate panes, and a body for the properties pane so its header and
    // the empty/selected content do not fight over the same container.
    var treeHead = el('header', 'mw-panel-head');
    treeHead.appendChild(el('span', 'mw-panel-title', 'Structure'));
    treePanel.appendChild(treeHead);

    var propsHead = el('header', 'mw-panel-head');
    propsHead.appendChild(el('span', 'mw-panel-title', 'Properties'));
    var propsBody = el('div', 'mw-props-body');
    propsPanel.appendChild(propsHead);
    propsPanel.appendChild(propsBody);

    // The context bar (project · version · commit) is page chrome, not model content, so it
    // stays above the three panes rather than being swallowed into the scrollable content pane.
    var contextBar = main.querySelector(':scope > .context-bar');
    if (contextBar) {
      contextBar.remove();
    }
    while (main.firstChild) {
      content.appendChild(main.firstChild);
    }
    workbench.appendChild(treePanel);
    workbench.appendChild(content);
    workbench.appendChild(propsPanel);
    if (contextBar) {
      main.appendChild(contextBar);
    }
    main.appendChild(workbench);

    var search = el('input', 'mw-search');
    search.type = 'search';
    search.placeholder = 'Filter name / id / stereotype';
    search.setAttribute('aria-label', 'Filter the containment tree');
    treePanel.appendChild(search);

    var state = buildTree(treePanel, propsBody);

    search.addEventListener('input', function () {
      applyFilter(state, search.value);
    });

    var initial = readSelectFromUrl();
    if (initial) {
      restoreSelection(state, initial);
    }

    wireKeyboard(state);
  }

  // A selectable node: the model data plus its place in the page and in the tree.
  function makeNode(domElement, section) {
    var node = {
      key: readAttr(domElement, 'mw-id'),
      name: readAttr(domElement, 'mw-name'),
      kind: readAttr(domElement, 'mw-kind'),
      section: section,
      domElement: domElement,
      children: [],
      parent: null,
      group: null,
      collapsed: false,
      visible: true,
      rowElement: null,
      toggleElement: null,
    };
    var stereotypes = readJson(domElement, 'mw-stereotypes');
    var attributes = readJson(domElement, 'mw-attributes');
    node.stereotypes = Array.isArray(stereotypes) ? stereotypes : [];
    node.attributes = Array.isArray(attributes) ? attributes : [];
    node.documentation = readAttr(domElement, 'mw-documentation');
    if (section === 'requirement') {
      node.reqId = readAttr(domElement, 'mw-reqid');
      node.reqText = readAttr(domElement, 'mw-reqtext');
      node.coverage = readAttr(domElement, 'mw-coverage');
    }
    return node;
  }

  function buildTree(treePanel, propsBody) {
    var tree = el('ul', 'mw-tree');
    treePanel.appendChild(tree);

    var state = {
      tree: tree,
      props: propsBody,
      flat: [],
      groups: [],
      selectedKey: null,
    };

    var structure = [];
    var structureUl = document.querySelector('.structure-tree');
    if (structureUl) {
      structure = collectStructureLevel(structureUl, state, null);
    }

    var requirements = collectFlat('tr.requirement[data-mw-id]', 'requirement', state);
    var signals = collectFlat('li.signal[data-mw-id]', 'signal', state);
    var interfaces = collectFlat('li.interface[data-mw-id]', 'interface', state);
    var activities = collectFlat('div.activity[data-mw-id]', 'activity', state);

    var groups = [
      { label: 'Structure', children: structure },
      { label: 'Requirements', children: requirements },
      { label: 'Signals', children: signals },
      { label: 'Interfaces', children: interfaces },
      { label: 'Activities', children: activities },
    ];

    each(groups, function (group) {
      if (group.children.length === 0) {
        return;
      }
      // The group element MUST be attached to the tree here. An earlier
      // version built it and discarded the return value, so the containment
      // tree rendered empty at runtime while every syntax check passed - the
      // browser test suite caught exactly that, and this append is what
      // makes the tree exist.
      tree.appendChild(buildGroup(tree, group, state));
      state.groups.push(group);
    });

    if (state.flat.length === 0) {
      tree.appendChild(el('li', 'mw-empty', 'This model has no elements.'));
    }

    renderEmptyProps(propsBody);
    return state;
  }

  function collectStructureLevel(ul, state, parent) {
    var nodes = [];
    each(ul.children, function (li) {
      if (li.tagName !== 'LI') {
        return;
      }
      var node = makeNode(li, 'structure');
      node.parent = parent;
      state.flat.push(node);
      var nested = childUl(li);
      if (nested) {
        node.children = collectStructureLevel(nested, state, node);
      }
      nodes.push(node);
    });
    return nodes;
  }

  function collectFlat(selector, section, state) {
    var nodes = [];
    each(document.querySelectorAll(selector), function (element) {
      var node = makeNode(element, section);
      state.flat.push(node);
      nodes.push(node);
    });
    return nodes;
  }

  function buildGroup(tree, group, state) {
    var li = el('li');
    var head = el('div', 'mw-group-label');
    var toggle = el('span', 'mw-toggle', '\u25be');
    head.appendChild(toggle);
    head.appendChild(document.createTextNode(group.label));
    head.addEventListener('click', function () {
      setCollapsed(group, !group.collapsed);
    });
    li.appendChild(head);

    var ul = el('ul');
    each(group.children, function (child) {
      child.group = group;
      ul.appendChild(buildNode(child, state));
    });
    li.appendChild(ul);

    group.liElement = li;
    group.toggleElement = toggle;
    group.collapsed = false;
    return li;
  }

  function buildNode(node, state) {
    var li = el('li');
    var row = el('div', 'mw-node');
    row.setAttribute('data-key', node.key);
    row.setAttribute('data-kind', node.kind);
    row.setAttribute('role', 'treeitem');
    row.setAttribute('tabindex', '-1');

    var toggle;
    if (node.children.length > 0) {
      toggle = el('span', 'mw-toggle', '\u25be');
      toggle.addEventListener('click', function (event) {
        event.stopPropagation();
        setCollapsed(node, !node.collapsed);
      });
    } else {
      toggle = el('span', 'mw-toggle', '');
    }
    row.appendChild(toggle);
    if (node.kind) {
      row.appendChild(el('span', 'mw-kind-dot'));
    }
    var nameEl = el('span', 'mw-node-name', node.name || node.key);
    // The row truncates long names with an ellipsis; the full name stays available on hover.
    nameEl.setAttribute('title', node.name || node.key);
    row.appendChild(nameEl);
    if (node.section === 'requirement') {
      // A requirement's coverage state, as a text chip (never colour alone).
      var cov = coverageBadge(node.coverage);
      cov.className += ' mw-tree-cov';
      row.appendChild(cov);
    } else if (node.kind) {
      row.appendChild(el('span', 'mw-kind-badge', node.kind));
    }

    row.addEventListener('click', function () {
      selectNode(state, node, true);
    });

    li.appendChild(row);
    node.liElement = li;
    node.rowElement = row;
    node.toggleElement = toggle;

    if (node.children.length > 0) {
      var ul = el('ul');
      each(node.children, function (child) {
        child.group = node.group;
        ul.appendChild(buildNode(child, state));
      });
      li.appendChild(ul);
    }
    return li;
  }

  function setCollapsed(item, collapsed) {
    item.collapsed = collapsed;
    var ul = childUl(item.liElement);
    if (ul) {
      ul.style.display = collapsed ? 'none' : '';
    }
    if (item.toggleElement) {
      item.toggleElement.textContent = collapsed ? '\u25b8' : '\u25be';
    }
  }

  // ---- selection and the properties panel ----------------------------------

  function selectNode(state, node, scrollPage) {
    var previous = state.tree.querySelector('.mw-node.mw-selected');
    if (previous) {
      previous.classList.remove('mw-selected');
    }
    clearPageHighlight();

    state.selectedKey = node.key;

    if (node.rowElement) {
      node.rowElement.classList.add('mw-selected');
      if (typeof node.rowElement.scrollIntoView === 'function') {
        node.rowElement.scrollIntoView({ block: 'nearest' });
      }
    }

    if (node.domElement) {
      node.domElement.classList.add('mw-highlight');
      if (scrollPage && typeof node.domElement.scrollIntoView === 'function') {
        node.domElement.scrollIntoView({ block: 'center', behavior: 'smooth' });
      }
    }

    renderProps(state.props, node);
    setSelectParam(node.key);
    syncDiagramLink(node.key);
  }

  function clearPageHighlight() {
    each(document.querySelectorAll('.mw-highlight'), function (element) {
      element.classList.remove('mw-highlight');
    });
  }

  function syncDiagramLink(key) {
    each(document.querySelectorAll('a[href*="/diagram"]'), function (link) {
      if (typeof URL === 'undefined') {
        return;
      }
      var url = new URL(link.href, window.location.href);
      url.searchParams.set('select', key);
      link.href = url.toString();
    });
  }

  function restoreSelection(state, key) {
    for (var i = 0; i < state.flat.length; i += 1) {
      if (state.flat[i].key === key) {
        expandTo(state.flat[i]);
        selectNode(state, state.flat[i], true);
        return;
      }
    }
  }

  function expandTo(node) {
    if (node.group) {
      setCollapsed(node.group, false);
    }
    var parent = node.parent;
    while (parent) {
      setCollapsed(parent, false);
      parent = parent.parent;
    }
  }

  function renderEmptyProps(body) {
    body.textContent = '';
    body.appendChild(el('h3', 'mw-props-title', 'Selection'));
    body.appendChild(el('p', 'mw-empty', 'Select an element to see its properties.'));
  }

  function renderProps(body, node) {
    body.textContent = '';

    body.appendChild(el('h3', 'mw-props-title', node.name || node.key));

    var dl = el('dl', 'mw-props-list');
    appendRow(dl, 'id', node.key);
    appendRow(dl, 'name', node.name);
    appendRow(dl, 'kind', node.kind);
    appendRow(
      dl,
      'stereotypes',
      node.stereotypes.length > 0 ? node.stereotypes.join(', ') : undefined
    );
    appendRow(dl, 'documentation', node.documentation);

    var attrTerm = el('dt', '', 'attributes');
    var attrDesc = el('dd');
    if (node.attributes.length > 0) {
      var list = el('ul', 'mw-attr-list');
      each(node.attributes, function (attribute) {
        var text = attribute.name || '';
        if (attribute.type) {
          text += ' : ' + attribute.type;
        }
        if (attribute.aggregation) {
          text += ' [' + attribute.aggregation + ']';
        }
        list.appendChild(el('li', 'mw-attr', text));
      });
      attrDesc.appendChild(list);
    } else {
      attrDesc.appendChild(el('span', 'mw-empty', '\u2014'));
    }
    dl.appendChild(attrTerm);
    dl.appendChild(attrDesc);

    if (node.section === 'requirement') {
      appendRow(dl, 'reqId', node.reqId);
      appendRow(dl, 'text', node.reqText);

      var covTerm = el('dt', '', 'coverage');
      var covDesc = el('dd');
      covDesc.appendChild(coverageBadge(node.coverage));
      dl.appendChild(covTerm);
      dl.appendChild(covDesc);
    }

    body.appendChild(dl);
  }

  function appendRow(dl, label, value) {
    var term = el('dt', '', label);
    var desc = el('dd');
    if (value === undefined || value === null || value === '') {
      desc.appendChild(el('span', 'mw-empty', '\u2014'));
    } else {
      desc.textContent = value;
    }
    dl.appendChild(term);
    dl.appendChild(desc);
  }

  function coverageBadge(coverage) {
    if (coverage === 'covered') {
      return el('span', 'mw-badge-covered', 'covered');
    }
    if (coverage === 'uncovered') {
      return el('span', 'mw-badge-uncovered', 'uncovered');
    }
    return el('span', 'mw-badge-unknown', 'not reported');
  }

  // ---- filter / search -----------------------------------------------------

  function applyFilter(state, query) {
    var q = query.trim().toLowerCase();

    each(state.groups, function (group) {
      var anyVisible = false;
      each(group.children, function (root) {
        if (computeVisibility(root, q)) {
          anyVisible = true;
        }
      });
      group.visible = anyVisible;
      group.liElement.style.display = q === '' || anyVisible ? '' : 'none';
      var ul = childUl(group.liElement);
      if (ul) {
        // While filtering, expand every group that still has a match so its matches are
        // visible; otherwise honour the engineer's collapse state.
        ul.style.display = q === '' ? (group.collapsed ? 'none' : '') : '';
      }
    });

    each(state.flat, function (node) {
      node.liElement.style.display = node.visible ? '' : 'none';
      var ul = childUl(node.liElement);
      if (ul && node.children.length > 0) {
        ul.style.display = q === '' ? (node.collapsed ? 'none' : '') : '';
      }
    });
  }

  function computeVisibility(node, q) {
    var childVisible = false;
    each(node.children, function (child) {
      if (computeVisibility(child, q)) {
        childVisible = true;
      }
    });
    node.visible = q === '' || nodeMatches(node, q) || childVisible;
    return node.visible;
  }

  function nodeMatches(node, q) {
    if (!q) {
      return true;
    }
    var haystack = [
      node.name,
      node.key,
      node.kind,
      node.section,
      node.reqId,
      node.reqText,
    ].join(' ');
    if (node.stereotypes.length > 0) {
      haystack += ' ' + node.stereotypes.join(' ');
    }
    return haystack.toLowerCase().indexOf(q) !== -1;
  }

  // ---- keyboard -------------------------------------------------------------

  function wireKeyboard(state) {
    document.addEventListener('keydown', function (event) {
      var active = document.activeElement;
      if (active && (active.tagName === 'INPUT' || active.tagName === 'TEXTAREA' || active.tagName === 'SELECT')) {
        return;
      }

      var visible = [];
      each(state.flat, function (node) {
        if (node.visible) {
          visible.push(node);
        }
      });
      if (visible.length === 0) {
        return;
      }

      var index = -1;
      for (var i = 0; i < visible.length; i += 1) {
        if (visible[i].key === state.selectedKey) {
          index = i;
          break;
        }
      }

      if (event.key === 'ArrowDown') {
        index = index < 0 ? 0 : Math.min(index + 1, visible.length - 1);
        selectNode(state, visible[index], false);
        event.preventDefault();
      } else if (event.key === 'ArrowUp') {
        index = index < 0 ? 0 : Math.max(index - 1, 0);
        selectNode(state, visible[index], false);
        event.preventDefault();
      } else if (event.key === 'Enter') {
        if (index >= 0) {
          selectNode(state, visible[index], true);
          event.preventDefault();
        }
      }
    });
  }
})();
