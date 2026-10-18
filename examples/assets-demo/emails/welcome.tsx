import { Body, Button, Container, Head, Heading, Html, Img, Link, Preview, Text } from "@react-email/components";

export const subject = "Welcome to Acme, your workspace is ready";

interface WelcomeProps {
  name: string;
  url: string;
}

export default function Welcome({ name, url }: WelcomeProps) {
  return (
    <Html>
      <Head />
      <Preview>Invite your team and ship your first project today.</Preview>
      <Body style={{ backgroundColor: "#f6f9fc", fontFamily: "Helvetica, Arial, sans-serif" }}>
        <Container style={{ backgroundColor: "#ffffff", padding: 32, borderRadius: 8 }}>
          <Img src="https://acme.example/logo.png" alt="Acme logo" width={120} height={32} />
          <Heading style={{ fontSize: 24, color: "#111827" }}>Welcome aboard, {name}!</Heading>
          <Text>Your workspace is ready. Invite your team and start shipping together.</Text>
          <Button
            href={url}
            style={{ backgroundColor: "#4f46e5", color: "#ffffff", padding: "12px 20px", borderRadius: 6 }}
          >
            Open my workspace
          </Button>
          <Text style={{ color: "#6b7280", fontSize: 12 }}>
            Questions? Read the <Link href="https://acme.example/docs">getting started guide</Link>.
          </Text>
        </Container>
      </Body>
    </Html>
  );
}

Welcome.PreviewProps = { name: "Ada", url: "https://acme.example/w/ada" } satisfies WelcomeProps;
