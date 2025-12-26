from flask import Flask, render_template, request, jsonify
from flask_sqlalchemy import SQLAlchemy
from datetime import datetime, timedelta

app = Flask(__name__)

# Config: SQLite database stored in the same folder
app.config['SQLALCHEMY_DATABASE_URI'] = 'sqlite:///database.db'
app.config['SQLALCHEMY_TRACK_MODIFICATIONS'] = False
db = SQLAlchemy(app)

def extract_name_from_url(url):
    """
    Parses 'https://leetcode.com/problems/two-sum/...' 
    into 'Two Sum'
    """
    try:
        # Remove trailing slash if present to avoid empty strings in split
        clean_url = url.rstrip('/')
        parts = clean_url.split('/')
        
        # Find the segment after 'problems'
        if 'problems' in parts:
            index = parts.index('problems') + 1
            if index < len(parts):
                raw_name = parts[index]
                # Convert 'two-sum' to 'Two Sum'
                return raw_name.replace('-', ' ').title()
    except:
        pass
    return "Unknown Problem"

# --- Database Model ---
class Problem(db.Model):
    id = db.Column(db.Integer, primary_key=True)
    name = db.Column(db.String(100), nullable=False)
    link = db.Column(db.String(200), nullable=False)
    due_date = db.Column(db.DateTime, default=datetime.now)
    repetitions_left = db.Column(db.Integer, default=3) # Default total reps
    current_interval = db.Column(db.Integer, default=1) # Days until next review

    def to_dict(self):
        return {
            "id": self.id,
            "name": self.name,
            "link": self.link,
            "due_date": self.due_date.strftime('%Y-%m-%d'),
            "is_due": datetime.now() >= self.due_date,
            "repetitions_left": self.repetitions_left
        }

# --- Routes ---

@app.route('/')
def index():
    return render_template('index.html')

@app.route('/api/problems', methods=['GET'])
def get_problems():
    # Sort by due date (overdue first)
    problems = Problem.query.order_by(Problem.due_date).all()
    return jsonify([p.to_dict() for p in problems])

@app.route('/api/add', methods=['POST'])
def add_problem():
    data = request.json
    name = extract_name_from_url(data['link'])
    new_problem = Problem(
        name=name,
        link=data['link'],
        repetitions_left=data.get('reps', 3), # Default 3 reps
        current_interval=14, # Start with 14 day interval
        due_date=datetime.now() + timedelta(days=14)
    )
    db.session.add(new_problem)
    db.session.commit()
    return jsonify({'message': 'Added successfully'})

@app.route('/api/review/<int:id>', methods=['POST'])
def review_problem(id):  # <--- Change made here: added 'id'
    """
    Update logic based on user result.
    'success' -> Increase interval exponentially, decrease reps left.
    'fail' -> Reset interval to 1 day, keep reps same (or reset reps).
    """
    data = request.json
    result = data.get('result') # 'success' or 'fail'
    problem = Problem.query.get_or_404(id)

    if result == 'success':
        problem.repetitions_left -= 1
        if problem.repetitions_left <= 0:
            db.session.delete(problem) # Completed!
            db.session.commit()
            return jsonify({'message': 'Problem completed and removed!'})
        
        # Exponential Backoff (Factor of 2)
        problem.current_interval = problem.current_interval * 2
        problem.due_date = datetime.now() + timedelta(days=problem.current_interval)
    
    else: # User forgot the solution
        problem.current_interval = 1 # Reset to tomorrow
        problem.due_date = datetime.now() + timedelta(days=1)
        # Optional: Reset repetitions_left to max if you want strict mastery
        # problem.repetitions_left = 3 

    db.session.commit()
    return jsonify({'message': 'Updated'})

@app.route('/api/delete/<int:id>', methods=['DELETE'])
def delete_problem(id):
    problem = Problem.query.get_or_404(id)
    db.session.delete(problem)
    db.session.commit()
    return jsonify({'message': 'Deleted'})

if __name__ == '__main__':
    with app.app_context():
        db.create_all() # Creates the DB file if it doesn't exist
    app.run(debug=True, port=5000)
