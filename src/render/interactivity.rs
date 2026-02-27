//! Inline JavaScript for hover/click visibility toggling (DESIGN.md §5.4).

/// Returns the complete inline JavaScript for an obgraph SVG.
pub fn js() -> &'static str {
    r#"(function() {
  var svg = document.currentScript.closest('.obgraph');
  if (!svg) return;

  var selected = new Set();
  var hovered = new Set();
  var hoveredProp = null;

  function hasSelectedParticipant(el) {
    var attr = el.getAttribute('data-participants');
    if (!attr) return false;
    var ids = attr.split(',');
    for (var i = 0; i < ids.length; i++) {
      if (selected.has(ids[i])) return true;
    }
    return false;
  }

  function hasHoveredParticipant(el) {
    var attr = el.getAttribute('data-participants');
    if (!attr) return false;
    var ids = attr.split(',');
    for (var i = 0; i < ids.length; i++) {
      if (hovered.has(ids[i])) return true;
    }
    return false;
  }

  function matchesHoveredProp(el) {
    if (!hoveredProp) return false;
    var attr = el.getAttribute('data-props');
    if (!attr) return false;
    var ids = attr.split(',');
    for (var i = 0; i < ids.length; i++) {
      if (ids[i] === hoveredProp) return true;
    }
    return false;
  }

  // An edge should be shown (full path visible, stub hidden) if:
  //   (a) any participant node is selected, OR
  //   (b) any participant node is hovered AND (no prop hover, or prop matches)
  function isEdgeVisible(el) {
    if (hasSelectedParticipant(el)) return true;
    if (!hasHoveredParticipant(el)) return false;
    if (hoveredProp === null) return true;
    return matchesHoveredProp(el);
  }

  function updateEdges() {
    svg.querySelectorAll('.obgraph-constraint-full').forEach(function(p) {
      if (isEdgeVisible(p)) {
        p.classList.add('obgraph-active');
      } else {
        p.classList.remove('obgraph-active');
      }
    });
    svg.querySelectorAll('.obgraph-constraint-stub').forEach(function(p) {
      if (isEdgeVisible(p)) {
        p.classList.add('obgraph-hidden');
      } else {
        p.classList.remove('obgraph-hidden');
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
      hoveredProp = null;
      updateEdges();
    });
    node.querySelectorAll('.obgraph-prop').forEach(function(prop) {
      prop.addEventListener('mouseenter', function() {
        hoveredProp = prop.getAttribute('data-prop');
        updateEdges();
      });
      prop.addEventListener('mouseleave', function() {
        hoveredProp = null;
        updateEdges();
      });
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
