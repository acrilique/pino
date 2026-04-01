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
            tableStartWidth: totalW
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
        resizing.th.style.width = newW + 'px';
        // Adjust table width so it grows/shrinks with the column
        if (table) {
            table.style.width = (resizing.tableStartWidth + (newW - resizing.startWidth)) + 'px';
        }
    });
    document.addEventListener('mouseup', function() {
        if (resizing) {
            resizing.handle.classList.remove('active');
            resizing = null;
            document.body.style.cursor = '';
            document.body.style.userSelect = '';
        }
    });
})()
