import { Circle, CheckCircle2, Clock } from "lucide-react";

interface TodoItem {
  content: string;
  status: "pending" | "in_progress" | "completed";
}

interface Props {
  output: string;
}

const statusIcon: Record<string, { icon: typeof Circle; className: string }> = {
  pending: {
    icon: Circle,
    className: "text-neutral-400 dark:text-neutral-500",
  },
  in_progress: { icon: Clock, className: "text-blue-500 dark:text-blue-400" },
  completed: {
    icon: CheckCircle2,
    className: "text-green-500 dark:text-green-400",
  },
};

function parseTodos(output: string): TodoItem[] {
  try {
    const parsed = JSON.parse(output);
    if (Array.isArray(parsed)) return parsed;
    if (parsed && Array.isArray(parsed.newTodos)) return parsed.newTodos;
    if (parsed && Array.isArray(parsed.todos)) return parsed.todos;
  } catch {
    // Not valid JSON
  }
  return [];
}

export function TodoRenderer({ output }: Props) {
  const todos = parseTodos(output);

  if (todos.length === 0) {
    return (
      <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-xs leading-relaxed text-neutral-600 dark:text-neutral-400">
        {output}
      </pre>
    );
  }

  return (
    <div className="space-y-1">
      {todos.map((todo, idx) => {
        const StatusIcon = statusIcon[todo.status]?.icon ?? Circle;
        const statusClass = statusIcon[todo.status]?.className ?? "";
        const isDone = todo.status === "completed";

        return (
          <div
            key={idx}
            className={`flex items-start gap-2 rounded-md px-2 py-1 text-sm ${
              isDone ? "opacity-60" : ""
            }`}
          >
            <StatusIcon className={`mt-0.5 h-4 w-4 flex-shrink-0 ${statusClass}`} />
            <div className="flex-1 min-w-0">
              <span
                className={
                  isDone
                    ? "text-neutral-500 dark:text-neutral-400"
                    : "text-neutral-800 dark:text-neutral-200"
                }
              >
                {todo.content}
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );
}
