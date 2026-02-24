//! Inline JavaScript for hover/click visibility toggling (DESIGN.md §5.4).

/// Returns the complete inline JavaScript for an obgraph SVG.
pub fn js() -> &'static str {
    r#"(function() {
  var svg = document.currentScript.closest('.obgraph');
  if (!svg) return;

  var selected = new Set();
  var hovered = new Set();

  function isActive(id) {
    return selected.has(id) || hovered.has(id);
  }

  function hasActiveParticipant(el) {
    var attr = el.getAttribute('data-participants');
    if (!attr) return false;
    var ids = attr.split(',');
    for (var i = 0; i < ids.length; i++) {
      if (isActive(ids[i])) return true;
    }
    return false;
  }

  function updateEdges() {
    svg.querySelectorAll('.obgraph-constraint-full').forEach(function(p) {
      if (hasActiveParticipant(p)) {
        p.classList.add('obgraph-active');
      } else {
        p.classList.remove('obgraph-active');
      }
    });
    svg.querySelectorAll('.obgraph-constraint-stub').forEach(function(p) {
      if (hasActiveParticipant(p)) {
        p.classList.add('obgraph-hidden');
      } else {
        p.classList.remove('obgraph-hidden');
      }
    });
    // Derivation chain atomic toggling
    svg.querySelectorAll('.obgraph-deriv-chain').forEach(function(g) {
      if (hasActiveParticipant(g)) {
        g.querySelectorAll('.obgraph-constraint-full').forEach(function(p) {
          p.classList.add('obgraph-active');
        });
        g.querySelectorAll('.obgraph-constraint-stub').forEach(function(p) {
          p.classList.add('obgraph-hidden');
        });
      } else {
        g.querySelectorAll('.obgraph-constraint-full').forEach(function(p) {
          p.classList.remove('obgraph-active');
        });
        g.querySelectorAll('.obgraph-constraint-stub').forEach(function(p) {
          p.classList.remove('obgraph-hidden');
        });
      }
    });
  }

  svg.querySelectorAll('.obgraph-node').forEach(function(node) {
    var id = node.getAttribute('data-node');
    if (node.getAttribute('data-selected') === 'true') {
      selected.add(id);
    }
    node.addEventListener('mouseenter', function() {
      hovered.add(id);
      updateEdges();
    });
    node.addEventListener('mouseleave', function() {
      hovered.delete(id);
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
