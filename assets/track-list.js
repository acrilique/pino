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

    var resizing = null;
    el.addEventListener('mousedown', function(e) {
        if (!e.target.classList.contains('col-resizer')) return;
        e.preventDefault();
        var th = e.target.closest('th');
        if (!th) return;
        var nextTh = th.nextElementSibling;
        if (!nextTh) return;
        var allThs = Array.from(th.parentElement.children);
        allThs.forEach(function(c) { c.style.width = c.offsetWidth + 'px'; });
        resizing = {
            th: th,
            handle: e.target,
            startX: e.pageX,
            startWidth: th.offsetWidth,
            nextTh: nextTh,
            nextWidth: nextTh ? nextTh.offsetWidth : 0
        };
        e.target.classList.add('active');
        document.body.style.cursor = 'col-resize';
        document.body.style.userSelect = 'none';
    });
    document.addEventListener('mousemove', function(e) {
        if (!resizing) return;
        e.preventDefault();
        var delta = e.pageX - resizing.startX;
        var maxGrow = resizing.nextTh ? resizing.nextWidth - 40 : 0;
        var maxShrink = resizing.startWidth - 40;
        delta = Math.max(-maxShrink, Math.min(delta, maxGrow));
        resizing.th.style.width = (resizing.startWidth + delta) + 'px';
        if (resizing.nextTh) {
            resizing.nextTh.style.width = (resizing.nextWidth - delta) + 'px';
        }
    });
    document.addEventListener('mouseup', function() {
        if (resizing) {
            resizing.handle.classList.remove('active');
            resizing = null;
            document.body.style.cursor = '';
            document.body.style.userSelect = '';
            var table = el.querySelector('table');
            if (table) {
                var ths = Array.from(table.querySelector('thead tr').children);
                var tableW = table.offsetWidth;
                if (tableW > 0) {
                    var pcts = ths.map(function(th) {
                        return (th.offsetWidth / tableW * 100).toFixed(2);
                    });
                    if (window.__pino_save_widths) window.__pino_save_widths(pcts.join(','));
                }
            }
        }
    });
})()
