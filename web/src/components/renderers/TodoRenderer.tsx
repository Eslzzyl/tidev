import { CheckCircle2, Circle, Clock } from "lucide-react";

interface TodoItem {
  content: string;
  status: "pending" | "in_progress" | "completed" | string;
}

function parseTodos(output: string): TodoItem[] {
  try {
    const value = JSON.parse(output) as unknown;
    if (Array.isArray(value)) return value as TodoItem[];
    if (value && typeof value === "object") {
      const object = value as { todos?: unknown; newTodos?: unknown };
      if (Array.isArray(object.todos)) return object.todos as TodoItem[];
      if (Array.isArray(object.newTodos)) return object.newTodos as TodoItem[];
    }
  } catch {
    // Keep the raw output when the tool did not return JSON.
  }
  return [];
}

export function TodoRenderer({ output }: { output: string }) {
  const todos = parseTodos(output);
  if (todos.length === 0) {
    return <pre className="tool-raw-output">{output}</pre>;
  }

  return (
    <div className="todo-renderer">
      {todos.map((todo, index) => {
        const completed = todo.status === "completed";
        const Icon = completed ? CheckCircle2 : todo.status === "in_progress" ? Clock : Circle;
        return (
          <div
            className={completed ? "todo-renderer-item completed" : "todo-renderer-item"}
            key={`${todo.content}-${index}`}
          >
            <Icon size={15} />
            <span>{todo.content}</span>
          </div>
        );
      })}
    </div>
  );
}
