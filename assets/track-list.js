(function() {
    const el = document.getElementById('track-list');
    if (!el) return;
    function resize() {
        const top = el.getBoundingClientRect().top;
        el.style.height = (window.innerHeight - top - 34) + 'px';
    }
    resize();
    window.__pino_resize = resize;
    window.addEventListener('resize', resize);

    var table = el.querySelector('table');
    var resizing = null;

    el.addEventListener('mousedown', function(e) {
        if (!e.target.classList.contains('col-resizer')) return;
        e.preventDefault();
        var th = e.target.closest('th');
        if (!th) return;

        // Snapshot all current widths as pixels so table-layout:fixed respects them
        var allThs = Array.from(th.parentElement.children);
        var totalW = allThs.reduce(function(s, c) { return s + c.offsetWidth; }, 0);
        allThs.forEach(function(c) { c.style.width = c.offsetWidth + 'px'; });
        // Set table width to total so adding width to one col can expand the table
        if (table) table.style.width = totalW + 'px';

        resizing = {
            th: th,
            handle: e.target,
            startX: e.pageX,
            startWidth: th.offsetWidth,
            tableStartWidth: totalW,
            containerWidth: el.clientWidth
        };
        e.target.classList.add('active');
        document.body.style.cursor = 'col-resize';
        document.body.style.userSelect = 'none';
    });
    document.addEventListener('mousemove', function(e) {
        if (!resizing) return;
        e.preventDefault();
        var delta = e.pageX - resizing.startX;
        var minW = 80;
        var newW = Math.max(minW, resizing.startWidth + delta);
        // Don't let the total table width shrink below the visible container width
        var newTableW = resizing.tableStartWidth + (newW - resizing.startWidth);
        if (newTableW < resizing.containerWidth) {
            newW = resizing.startWidth - (resizing.tableStartWidth - resizing.containerWidth);
            newW = Math.max(minW, newW);
            newTableW = resizing.containerWidth;
        }
        resizing.th.style.width = newW + 'px';
        if (table) {
            table.style.width = newTableW + 'px';
        }
        // Clamp scroll so no empty space appears on the right
        var maxScroll = table.offsetWidth - el.clientWidth;
        if (maxScroll < 0) maxScroll = 0;
        if (el.scrollLeft > maxScroll) el.scrollLeft = maxScroll;
    });
    document.addEventListener('mouseup', function() {
        if (resizing) {
            resizing.handle.classList.remove('active');
            resizing = null;
            document.body.style.cursor = '';
            document.body.style.userSelect = '';
        }
    });

    // ── Column drag reorder ──────────────────────────────────────────────────
    var dragging = null;
    var indicator = document.createElement('div');
    indicator.className = 'col-drag-indicator';
    indicator.style.display = 'none';
    document.body.appendChild(indicator);

    var ghost = document.createElement('div');
    ghost.className = 'col-drag-ghost';
    ghost.style.display = 'none';
    document.body.appendChild(ghost);

    el.addEventListener('mousedown', function(e) {
        if (resizing) return;
        var th = e.target.closest('thead th');
        if (!th || e.target.classList.contains('col-resizer') || e.button !== 0) return;
        var colId = th.getAttribute('data-col');
        if (!colId) return;
        e.preventDefault(); // prevent native drag & text selection
        dragging = {
            th: th,
            colId: colId,
            startX: e.clientX,
            startY: e.clientY,
            active: false
        };
    });

    document.addEventListener('mousemove', function(e) {
        if (!dragging) return;
        // Require a 5px move to start the drag
        if (!dragging.active) {
            var dx = e.clientX - dragging.startX;
            var dy = e.clientY - dragging.startY;
            if (Math.abs(dx) < 5 && Math.abs(dy) < 5) return;
            dragging.active = true;
            dragging.th.classList.add('drag-source');
            ghost.textContent = dragging.th.textContent.trim();
            ghost.style.display = 'block';
            document.body.style.cursor = 'grabbing';
            document.body.style.userSelect = 'none';
        }
        e.preventDefault();
        ghost.style.left = (e.clientX + 12) + 'px';
        ghost.style.top = (e.clientY - 12) + 'px';

        // Find target th by bounding rect hit test (elementFromPoint unreliable in webview)
        var allThs = Array.from(el.querySelectorAll('thead th'));
        var targetTh = null;
        for (var i = 0; i < allThs.length; i++) {
            var r = allThs[i].getBoundingClientRect();
            if (e.clientX >= r.left && e.clientX < r.right && e.clientY >= r.top && e.clientY < r.bottom) {
                targetTh = allThs[i];
                break;
            }
        }
        // Remove old indicators
        allThs.forEach(function(h) { h.classList.remove('drag-over-left', 'drag-over-right'); });
        indicator.style.display = 'none';

        if (targetTh && targetTh !== dragging.th && targetTh.getAttribute('data-col')) {
            var rect = targetTh.getBoundingClientRect();
            var midX = rect.left + rect.width / 2;
            var side = e.clientX < midX ? 'left' : 'right';
            targetTh.classList.add(side === 'left' ? 'drag-over-left' : 'drag-over-right');
            dragging.targetCol = targetTh.getAttribute('data-col');
            dragging.targetSide = side;

            // Show indicator line
            var lineX = side === 'left' ? rect.left : rect.right;
            indicator.style.display = '';
            indicator.style.left = (lineX + window.scrollX - 1) + 'px';
            indicator.style.top = (rect.top + window.scrollY) + 'px';
            indicator.style.height = rect.height + 'px';
        } else {
            dragging.targetCol = null;
        }
    });

    document.addEventListener('mouseup', function() {
        if (!dragging) return;
        if (dragging.active) {
            dragging.th.classList.remove('drag-source');
            ghost.style.display = 'none';
            indicator.style.display = 'none';
            document.body.style.userSelect = '';
            var allThs = Array.from(el.querySelectorAll('thead th'));
            allThs.forEach(function(h) { h.classList.remove('drag-over-left', 'drag-over-right'); });

            if (dragging.targetCol && dragging.targetCol !== dragging.colId) {
                var inp = document.getElementById('col-reorder-input');
                if (inp) {
                    var nativeSetter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
                    nativeSetter.call(inp, dragging.colId + ',' + dragging.targetCol + ',' + dragging.targetSide);
                    inp.dispatchEvent(new Event('input', { bubbles: true }));
                }
            }
            document.body.style.cursor = '';
        }
        dragging = null;
    });
})()
