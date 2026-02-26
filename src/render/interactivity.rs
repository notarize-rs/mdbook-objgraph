//! Inline JavaScript for hover/click visibility toggling (DESIGN.md §5.4).

/// Returns the complete inline JavaScript for an obgraph SVG.
pub fn js() -> &'static str {
    r#"(function() {
  var svg = document.currentScript.closest('.obgraph');
  if (!svg) return;

  var selected = new Set();
  var hoveredNode = null;
  var hoveredProp = null;

  function isNodeActive(id) {
    return selected.has(id) || hoveredNode === id;
  }

  // Check if any participant node is selected or title-bar-hovered.
  function hasActiveParticipant(el) {
    var attr = el.getAttribute('data-participants');
    if (!attr) return false;
    var ids = attr.split(',');
    for (var i = 0; i < ids.length; i++) {
      if (isNodeActive(ids[i])) return true;
    }
    return false;
  }

  // Check if any participant node is in the selected set.
  function hasSelectedParticipant(el) {
    var attr = el.getAttribute('data-participants');
    if (!attr) return false;
    var ids = attr.split(',');
    for (var i = 0; i < ids.length; i++) {
      if (selected.has(ids[i])) return true;
    }
    return false;
  }

  // Check if the hovered property matches any prop on this edge.
  function hasHoveredProp(el) {
    if (!hoveredProp) return false;
    var attr = el.getAttribute('data-props');
    if (!attr) return false;
    var ids = attr.split(',');
    for (var i = 0; i < ids.length; i++) {
      if (ids[i] === hoveredProp) return true;
    }
    return false;
  }

  // An edge is "active" (should be shown) if:
  //   (a) any participant node is selected, OR
  //   (b) hoveredNode is a participant AND either no prop hover or prop matches
  function isEdgeActive(el) {
    if (hasSelectedParticipant(el)) return true;
    if (hoveredNode !== null && hasActiveParticipant(el)) {
      if (hoveredProp === null) return true;
      return hasHoveredProp(el);
    }
    return false;
  }

  function updateEdges() {
    // Cross-domain full paths (skip those inside deriv-chain groups)
    svg.querySelectorAll('.obgraph-constraint-full').forEach(function(p) {
      if (p.closest('.obgraph-deriv-chain')) return;
      if (isEdgeActive(p)) {
        p.classList.add('obgraph-active');
      } else {
        p.classList.remove('obgraph-active');
      }
    });

    // Cross-domain stubs (skip those inside deriv-chain groups)
    svg.querySelectorAll('.obgraph-constraint-stub').forEach(function(p) {
      if (p.closest('.obgraph-deriv-chain')) return;
      if (isEdgeActive(p)) {
        p.classList.add('obgraph-hidden');
      } else {
        p.classList.remove('obgraph-hidden');
      }
    });

    // Derivation chain atomic toggling
    svg.querySelectorAll('.obgraph-deriv-chain').forEach(function(g) {
      var active = isEdgeActive(g);
      g.querySelectorAll('.obgraph-constraint-full').forEach(function(p) {
        if (active) {
          p.classList.add('obgraph-active');
        } else {
          p.classList.remove('obgraph-active');
        }
      });
      g.querySelectorAll('.obgraph-constraint-stub').forEach(function(p) {
        if (active) {
          p.classList.add('obgraph-hidden');
        } else {
          p.classList.remove('obgraph-hidden');
        }
      });
    });
  }

  svg.querySelectorAll('.obgraph-node').forEach(function(node) {
    var id = node.getAttribute('data-node');
    if (node.getAttribute('data-selected') === 'true') {
      selected.add(id);
    }

    // mouseover bubbles: detects property vs title-bar hover
    node.addEventListener('mouseover', function(e) {
      hoveredNode = id;
      var propEl = e.target.closest('.obgraph-prop');
      if (propEl) {
        hoveredProp = propEl.getAttribute('data-prop');
      } else {
        hoveredProp = null;
      }
      updateEdges();
    });

    node.addEventListener('mouseleave', function() {
      hoveredNode = null;
      hoveredProp = null;
      updateEdges();
    });

    node.addEventListener('click', function(e) {
      e.stopPropagation();
      if (selected.has(id)) {
        selected.delete(id);
        node.setAttribute('data-selected', 'false');
      } else {
        selected.add(id);
        node.setAttribute('data-selected', 'true');
      }
      updateEdges();
    });
  });

  svg.addEventListener('click', function() {
    selected.clear();
    svg.querySelectorAll('.obgraph-node').forEach(function(node) {
      node.setAttribute('data-selected', 'false');
    });
    updateEdges();
  });

  updateEdges();
})();"#
}
