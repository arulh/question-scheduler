async function loadProblems() {
    const res = await fetch('/api/problems');
    const problems = await res.json();
    
    const dueList = document.getElementById('dueList');
    const upcomingList = document.getElementById('upcomingList');
    
    dueList.innerHTML = '';
    upcomingList.innerHTML = '';

    problems.forEach(p => {
        const div = document.createElement('div');
        // Add class based on status
        div.className = `problem-item ${p.is_due ? 'is-due' : 'is-upcoming'}`;
        
        // Buttons HTML (Only show Pass/Fail if it is Due)
        let actionsHtml = '';
        if (p.is_due) {
            actionsHtml = `
                <div class="actions">
                    <button class="btn-fail" onclick="review(${p.id}, 'fail')">Forgot (Reset)</button>
                    <button class="btn-pass" onclick="review(${p.id}, 'success')">Solved (Next lvl)</button>
                </div>
            `;
        }

        div.innerHTML = `
            <div>
                <a href="${p.link}" target="_blank">${p.name} 🔗</a>
            </div>
            <div class="meta">
                <span>📅 Due: ${p.due_date}</span>
                <span>🔄 Rem: ${p.repetitions_left}</span>
                <span style="cursor:pointer; color:red;" onclick="deleteProblem(${p.id})">✖</span>
            </div>
            ${actionsHtml}
        `;

        if (p.is_due) {
            dueList.appendChild(div);
        } else {
            upcomingList.appendChild(div);
        }
    });
}

async function addProblem() {
    // Only get Link and Reps now
    const link = document.getElementById('pLink').value;
    const reps = document.getElementById('pReps').value;

    if(!link) return alert("Please enter a link");

    await fetch('/api/add', {
        method: 'POST',
        headers: {'Content-Type': 'application/json'},
        body: JSON.stringify({ 
            link: link, 
            reps: parseInt(reps) 
            // Name is no longer sent; backend parses it.
        })
    });
    
    document.getElementById('pLink').value = '';
    loadProblems();
}

async function review(id, result) {
    await fetch(`/api/review/${id}`, {
        method: 'POST',
        headers: {'Content-Type': 'application/json'},
        body: JSON.stringify({ result })
    });
    loadProblems();
}

async function deleteProblem(id) {
    if(confirm("Delete this problem?")) {
        await fetch(`/api/delete/${id}`, { method: 'DELETE' });
        loadProblems();
    }
}

// Initial Load
loadProblems();
