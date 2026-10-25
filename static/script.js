'use strict';

const MAX_INTERVAL = 365; // keep in sync with schedule.rs
const DAY_MS = 86_400_000;
const PREFS_KEY = 'qs-add-prefs';

// openDetails maps a problem id to its unsaved edits (or null), so re-renders keep drafts.
const state = { problems: [], stats: null, filter: '', openDetails: new Map() };

// --- Helpers ---

function el(tag, props = {}, ...children) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(props)) {
        if (value == null || value === false) continue;
        if (key === 'class') node.className = value;
        else if (key === 'value') node.value = value;
        else if (key.startsWith('on')) node.addEventListener(key.slice(2), value);
        else node.setAttribute(key, value === true ? '' : value);
    }
    for (const child of children.flat()) {
        if (child != null && child !== false) node.append(child);
    }
    return node;
}

async function api(method, path, body) {
    const res = await fetch(path, {
        method,
        headers: body ? { 'Content-Type': 'application/json' } : {},
        body: body ? JSON.stringify(body) : undefined,
    });
    let data = null;
    try { data = await res.json(); } catch { /* empty or non-JSON body */ }
    if (!res.ok) throw new Error(data?.error || `Request failed (${res.status})`);
    return data;
}

function parseDate(ymd) {
    const [y, m, d] = ymd.split('-').map(Number);
    return new Date(y, m - 1, d);
}

function today() {
    const now = new Date();
    return new Date(now.getFullYear(), now.getMonth(), now.getDate());
}

function addDays(date, days) {
    const d = new Date(date);
    d.setDate(d.getDate() + days);
    return d;
}

function daysBetween(from, to) {
    return Math.round((to - from) / DAY_MS);
}

function fmtDate(date) {
    const opts = { month: 'short', day: 'numeric' };
    if (date.getFullYear() !== new Date().getFullYear()) opts.year = 'numeric';
    return date.toLocaleDateString(undefined, opts);
}

function plural(n, word) {
    return `${n} ${word}${n === 1 ? '' : 's'}`;
}

/** 3 -> '3 days', 20 -> '3 weeks', 95 -> '3 months' */
function duration(days) {
    days = Math.abs(days);
    if (days < 14) return plural(days, 'day');
    if (days < 60) return plural(Math.round(days / 7), 'week');
    if (days < 730) return plural(Math.round(days / 30), 'month');
    return plural(Math.round(days / 365), 'year');
}

function shortDuration(days) {
    if (days < 14) return `${days}d`;
    if (days < 60) return `${Math.round(days / 7)}w`;
    return `${Math.round(days / 30)}mo`;
}

function capitalize(s) {
    return s.charAt(0).toUpperCase() + s.slice(1);
}

// --- Scheduling (mirrors schedule.rs, used for previews only) ---

function outcomeFor(p, result) {
    if (result === 'forgot') return { days: 1 };
    if (result === 'hard') {
        return { days: p.relearning ? Math.max(1, Math.floor(p.base_interval / 2)) : p.current_interval };
    }
    if (p.repetitions_left <= 1) return { mastered: true };
    const next = p.relearning ? p.base_interval : p.current_interval * 2;
    return { days: Math.min(MAX_INTERVAL, Math.max(1, next)) };
}

function plannedReviews(firstInterval, reps) {
    const dates = [];
    let date = today();
    let gap = firstInterval;
    for (let i = 0; i < reps; i++) {
        date = addDays(date, gap);
        dates.push(date);
        gap = Math.min(gap * 2, MAX_INTERVAL);
    }
    return dates;
}

// --- Toasts ---

function toast(message, { error = false, action, timeout = 5000 } = {}) {
    const node = el('div', { class: `toast${error ? ' error' : ''}`, role: error ? 'alert' : 'status' },
        el('span', {}, message));
    if (action) {
        node.append(el('button', {
            type: 'button',
            onclick: async () => {
                node.remove();
                await action.run();
            },
        }, action.label));
    }
    document.getElementById('toasts').append(node);
    setTimeout(() => node.remove(), action ? timeout + 3000 : timeout);
}

function showError(err) {
    toast(err.message || String(err), { error: true });
}

// --- Data ---

async function load() {
    try {
        const [problems, stats] = await Promise.all([api('GET', '/api/problems'), api('GET', '/api/stats')]);
        state.problems = problems;
        state.stats = stats;
        render();
    } catch (err) {
        showError(err);
    }
}

async function run(fn) {
    try {
        await fn();
    } catch (err) {
        showError(err);
    }
    await load();
}

// --- Actions ---

function review(p, result) {
    run(async () => {
        const updated = await api('POST', `/api/problems/${p.id}/review`, { result });
        let message;
        if (updated.status === 'mastered') {
            message = `Mastered ${p.name}!`;
        } else {
            const due = parseDate(updated.due_date);
            const when = updated.days_until_due === 1 ? 'tomorrow' : `in ${duration(updated.days_until_due)} (${fmtDate(due)})`;
            message = result === 'forgot'
                ? `Back ${when}, then every ${duration(updated.base_interval)} again`
                : `Next review ${when}`;
        }
        toast(message, { action: { label: 'Undo', run: () => run(() => api('POST', `/api/problems/${p.id}/undo`)) } });
    });
}

function snooze(p, days = 1) {
    run(async () => {
        const updated = await api('POST', `/api/problems/${p.id}/snooze`, { days });
        toast(`${p.name} moved to ${fmtDate(parseDate(updated.due_date))}`);
    });
}

function remove(p) {
    if (!confirm(`Delete "${p.name}" and its review history?`)) return;
    run(async () => {
        await api('DELETE', `/api/problems/${p.id}`);
        state.openDetails.delete(p.id);
        toast(`Deleted ${p.name}`);
    });
}

function reactivate(p) {
    run(async () => {
        await api('POST', `/api/problems/${p.id}/reactivate`);
        toast(`${p.name} is due again today`);
    });
}

function saveDetails(p, fields) {
    run(async () => {
        await api('PATCH', `/api/problems/${p.id}`, fields);
        state.openDetails.delete(p.id);
        toast('Saved');
    });
}

function spreadBacklog(due, perDay) {
    run(async () => {
        const moves = due
            .map((p, i) => ({ p, days: Math.floor(i / perDay) }))
            .filter(m => m.days > 0);
        if (!moves.length) {
            toast('Nothing to move, that already fits in one day');
            return;
        }
        await Promise.all(moves.map(m => api('POST', `/api/problems/${m.p.id}/snooze`, { days: m.days })));
        const lastDay = Math.max(...moves.map(m => m.days));
        toast(`Spread ${plural(due.length, 'problem')} over the next ${plural(lastDay + 1, 'day')}`);
    });
}

// --- Rendering ---

function render() {
    const query = state.filter.trim().toLowerCase();
    const visible = state.problems.filter(p => !query || p.name.toLowerCase().includes(query));
    const all = state.problems;

    // Most overdue first, then due today.
    const due = visible.filter(p => p.status === 'overdue' || p.status === 'due')
        .sort((a, b) => a.days_until_due - b.days_until_due);
    const upcoming = visible.filter(p => p.status === 'upcoming')
        .sort((a, b) => a.days_until_due - b.days_until_due);
    const mastered = visible.filter(p => p.status === 'mastered')
        .sort((a, b) => (b.mastered_at || '').localeCompare(a.mastered_at || ''));

    renderStats(all);
    document.getElementById('dueCount').textContent = due.length;
    document.getElementById('upcomingCount').textContent = upcoming.length;
    document.getElementById('masteredCount').textContent = mastered.length;
    document.getElementById('masteredSection').hidden = !all.some(p => p.status === 'mastered');

    renderBacklog(query ? [] : due);

    renderList('dueList', due, query
        ? 'No due problems match your filter.'
        : 'Nothing due. Add a problem or enjoy the break.');
    renderList('upcomingList', upcoming, query ? 'No upcoming problems match your filter.' : 'No upcoming reviews yet.',
        p => p.days_until_due <= 7 ? 'This week' : p.days_until_due <= 30 ? 'This month' : 'Later');
    renderList('masteredList', mastered, 'No mastered problems match your filter.');
}

function renderStats(problems) {
    const count = pred => problems.filter(pred).length;
    const overdue = count(p => p.status === 'overdue');
    const dueNow = overdue + count(p => p.status === 'due');
    const week = count(p => p.status === 'upcoming' && p.days_until_due <= 7);
    const masteredCount = count(p => p.status === 'mastered');
    const { streak = 0, reviewed_today: reviewedToday = 0 } = state.stats || {};

    const tile = (value, label, cls = '', title) =>
        el('div', { class: `stat ${cls}`, title }, el('div', { class: 'value' }, String(value)), el('div', { class: 'label' }, label));

    document.getElementById('stats').replaceChildren(
        tile(dueNow, 'Due now', dueNow ? 'warn' : ''),
        tile(overdue, 'Overdue', overdue ? 'danger' : ''),
        tile(week, 'Next 7 days'),
        tile(masteredCount, 'Mastered'),
        tile(`${streak}d`, reviewedToday ? `Streak · ${reviewedToday} today` : 'Streak', '',
            'Consecutive days with at least one review'),
    );
}

function renderBacklog(due) {
    const container = document.getElementById('backlog');
    const overdue = due.filter(p => p.status === 'overdue').length;
    if (overdue < 4) {
        container.replaceChildren();
        return;
    }
    const perDay = el('input', { type: 'number', min: 1, max: 20, value: 3, 'aria-label': 'Problems per day' });
    container.replaceChildren(el('div', { class: 'backlog' },
        el('span', {}, `${plural(overdue, 'problem')} overdue. Keep the most overdue today and push the rest back?`),
        el('div', { class: 'inline' },
            perDay,
            el('span', { class: 'muted' }, 'per day'),
            el('button', {
                type: 'button',
                class: 'btn secondary',
                onclick: () => {
                    const n = Math.max(1, Math.min(20, parseInt(perDay.value, 10) || 3));
                    spreadBacklog(due, n);
                },
            }, 'Spread out'),
        ),
    ));
}

function renderList(id, problems, emptyText, groupOf) {
    const list = document.getElementById(id);
    if (!problems.length) {
        list.replaceChildren(el('div', { class: 'empty' }, emptyText));
        return;
    }
    const nodes = [];
    let lastGroup = null;
    for (const p of problems) {
        const group = groupOf?.(p);
        if (group && group !== lastGroup) {
            nodes.push(el('div', { class: 'group-label' }, group));
            lastGroup = group;
        }
        nodes.push(problemCard(p));
    }
    list.replaceChildren(...nodes);
}

function statusPill(p) {
    const due = parseDate(p.due_date);
    const d = p.days_until_due;
    switch (p.status) {
        case 'overdue':
            return el('span', { class: 'pill overdue', title: `Was due ${fmtDate(due)}` }, `Overdue ${duration(d)}`);
        case 'due':
            return el('span', { class: 'pill due' }, 'Due today');
        case 'mastered':
            return el('span', { class: 'pill mastered' }, `Mastered ${fmtDate(parseDate(p.mastered_at || p.due_date))}`);
        default:
            return el('span', { class: 'pill', title: fmtDate(due) },
                d === 1 ? 'Tomorrow' : `In ${duration(d)} · ${fmtDate(due)}`);
    }
}

function lastReviewed(p) {
    const last = p.history.at(-1);
    if (last) {
        const ago = daysBetween(parseDate(last.date), today());
        return ago === 0 ? 'Reviewed today' : `Last reviewed ${duration(ago)} ago`;
    }
    return p.created_at ? `Added ${fmtDate(parseDate(p.created_at))}` : 'Not reviewed yet';
}

function problemCard(p) {
    const isMastered = p.status === 'mastered';
    const isDue = p.status === 'overdue' || p.status === 'due';
    const done = Math.max(0, p.total_reps - p.repetitions_left);

    const head = el('div', { class: 'problem-head' },
        el('div', { class: 'title-wrap' },
            el('a', { class: 'title', href: p.link, target: '_blank', rel: 'noopener noreferrer' },
                p.name, el('span', { class: 'ext', 'aria-hidden': 'true' }, '↗')),
            p.difficulty && el('span', { class: `badge ${p.difficulty}` }, capitalize(p.difficulty)),
        ),
        statusPill(p),
    );

    const progress = el('span', { title: `${done} of ${p.total_reps} successful reviews` },
        el('span', { class: 'progress' },
            Array.from({ length: p.total_reps }, (_, i) => el('i', { class: i < done ? 'on' : '' }))),
        `${done}/${p.total_reps}`);

    const gap = isMastered ? null
        : el('span', { title: 'Current gap between reviews' },
            p.relearning ? 'Relearning' : `Every ${duration(p.current_interval)}`);

    const history = p.history.length
        ? el('span', { class: 'history', title: p.history.map(h => `${h.date}: ${h.result}`).join('\n') },
            p.history.slice(-12).map(h => el('i', { class: h.result })))
        : null;

    const meta = el('div', { class: 'meta' }, progress, gap, el('span', {}, lastReviewed(p)), history);

    const detailsOpen = state.openDetails.has(p.id);
    const toggleDetails = () => {
        if (state.openDetails.has(p.id)) state.openDetails.delete(p.id);
        else state.openDetails.set(p.id, null);
        render();
    };

    const tools = el('div', { class: 'tools' },
        el('button', {
            type: 'button',
            class: `btn ghost${p.notes ? ' has-notes' : ''}`,
            'aria-expanded': String(detailsOpen),
            title: p.notes ? 'Show notes (try solving first!)' : 'Add notes or edit',
            onclick: toggleDetails,
        }, 'Notes'),
        !isMastered && el('button', { type: 'button', class: 'btn ghost', title: 'Push back one day', onclick: () => snooze(p) }, 'Snooze'),
        isMastered && el('button', { type: 'button', class: 'btn ghost', title: 'Bring back for one more review today', onclick: () => reactivate(p) }, 'Practice again'),
        el('button', { type: 'button', class: 'btn ghost danger', onclick: () => remove(p) }, 'Delete'),
    );

    const reviewButton = (result, label) => {
        const next = outcomeFor(p, result);
        const hint = next.mastered ? 'Mastered' : next.days === 1 ? 'Tomorrow' : shortDuration(next.days);
        return el('button', {
            type: 'button',
            class: `btn review ${result}`,
            title: next.mastered ? 'Last review: marks it mastered' : `Next review in ${duration(next.days)}`,
            onclick: e => {
                e.currentTarget.disabled = true;
                review(p, result);
            },
        }, label, el('small', {}, hint));
    };

    const actions = el('div', { class: 'actions' }, tools,
        isDue && el('div', { class: 'reviews' },
            reviewButton('forgot', 'Forgot'),
            reviewButton('hard', 'Hard'),
            reviewButton('solved', 'Solved'),
        ),
    );

    return el('article', { class: `problem ${p.status}` }, head, meta, actions, detailsOpen && detailsPanel(p, toggleDetails));
}

function detailsPanel(p, close) {
    const draft = state.openDetails.get(p.id) ?? { name: p.name, difficulty: p.difficulty || '', notes: p.notes };
    const name = el('input', { type: 'text', value: draft.name, 'aria-label': 'Name' });
    const difficulty = el('select', { 'aria-label': 'Difficulty' },
        ['', 'easy', 'medium', 'hard'].map(d =>
            el('option', { value: d, selected: draft.difficulty === d }, d ? capitalize(d) : 'No difficulty')));
    const notes = el('textarea', {
        class: 'notes',
        rows: 4,
        placeholder: 'Key insight, pattern, edge cases, complexity…',
        value: draft.notes,
        'aria-label': 'Notes',
    });
    const stamps = [
        p.created_at && `Added ${fmtDate(parseDate(p.created_at))}`,
        `first gap ${duration(p.base_interval)}`,
        p.history.length && plural(p.history.length, 'review'),
    ].filter(Boolean).join(' · ');

    const panel = el('form', {
        class: 'details',
        onsubmit: e => {
            e.preventDefault();
            saveDetails(p, { name: name.value, difficulty: difficulty.value, notes: notes.value });
        },
    },
        name, difficulty, notes,
        el('div', { class: 'stamp' }, stamps),
        el('div', { class: 'save-row' },
            el('button', { type: 'button', class: 'btn ghost', onclick: close }, 'Cancel'),
            el('button', { type: 'submit', class: 'btn primary' }, 'Save')),
    );
    const keepDraft = () => state.openDetails.set(p.id,
        { name: name.value, difficulty: difficulty.value, notes: notes.value });
    panel.addEventListener('input', keepDraft);
    panel.addEventListener('change', keepDraft);
    panel.addEventListener('keydown', e => {
        if (e.key === 'Escape') close();
        if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) panel.requestSubmit();
    });
    return panel;
}

// --- Add form ---

const form = document.getElementById('addForm');
const intervalInput = document.getElementById('firstInterval');
const repsInput = document.getElementById('reps');

function loadPrefs() {
    try {
        const prefs = JSON.parse(localStorage.getItem(PREFS_KEY) || '{}');
        if (prefs.firstInterval) intervalInput.value = prefs.firstInterval;
        if (prefs.reps) repsInput.value = prefs.reps;
    } catch { /* storage unavailable */ }
}

function savePrefs() {
    try {
        localStorage.setItem(PREFS_KEY, JSON.stringify({ firstInterval: intervalInput.value, reps: repsInput.value }));
    } catch { /* storage unavailable */ }
}

function updatePreview() {
    const first = parseInt(intervalInput.value, 10);
    const reps = parseInt(repsInput.value, 10);
    for (const chip of document.querySelectorAll('#intervalChips button')) {
        chip.classList.toggle('active', Number(chip.dataset.days) === first);
    }
    const preview = document.getElementById('preview');
    if (!(first >= 1 && first <= MAX_INTERVAL && reps >= 1 && reps <= 20)) {
        preview.textContent = `First review must be 1–${MAX_INTERVAL} days, reviews 1–20.`;
        return;
    }
    const dates = plannedReviews(first, reps).map(fmtDate);
    const shown = dates.length > 5 ? [...dates.slice(0, 4), '…', dates.at(-1)] : dates;
    preview.replaceChildren('If you solve it each time: ', el('strong', {}, shown.join(' → ')), ', then mastered.');
}

document.getElementById('intervalChips').addEventListener('click', e => {
    const chip = e.target.closest('button[data-days]');
    if (!chip) return;
    intervalInput.value = chip.dataset.days;
    updatePreview();
    savePrefs();
});

for (const input of [intervalInput, repsInput]) {
    input.addEventListener('input', () => {
        updatePreview();
        savePrefs();
    });
}

form.addEventListener('submit', async e => {
    e.preventDefault();
    const submit = form.querySelector('button[type="submit"]');
    submit.disabled = true;
    try {
        const added = await api('POST', '/api/problems', {
            link: document.getElementById('link').value,
            name: document.getElementById('name').value,
            difficulty: form.elements.difficulty.value,
            notes: document.getElementById('notes').value,
            reps: parseInt(repsInput.value, 10),
            first_interval: parseInt(intervalInput.value, 10),
        });
        for (const id of ['link', 'name', 'notes']) document.getElementById(id).value = '';
        form.elements.difficulty.value = '';
        form.querySelector('.notes-toggle').open = false;
        toast(`Added ${added.name}. First review ${fmtDate(parseDate(added.due_date))}`);
        await load();
    } catch (err) {
        showError(err);
    } finally {
        submit.disabled = false;
        document.getElementById('link').focus();
    }
});

document.getElementById('search').addEventListener('input', e => {
    state.filter = e.target.value;
    render();
});

// Due dates change at midnight; refresh when coming back to the tab.
document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') load();
});
setInterval(load, 30 * 60 * 1000);

loadPrefs();
updatePreview();
load();
