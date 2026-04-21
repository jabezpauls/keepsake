import { useSession } from "./state/session";
import Unlock from "./ui/unlock/Unlock";
import Shell from "./ui/shell/Shell";

export default function App() {
    const session = useSession((s) => s.session);
    return session ? <Shell /> : <Unlock />;
}
