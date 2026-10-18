import { Body, Button, Container, Head, Heading, Html, Preview, Section, Text } from "@react-email/components";

export const subject = "Payment received, thank you";

interface InvoicePaidProps {
  locale: string;
  dir: "ltr" | "rtl";
  amount: string;
  seats: number;
  invoiceUrl: string;
}

export default function InvoicePaid({ locale, dir, amount, seats, invoiceUrl }: InvoicePaidProps) {
  return (
    <Html lang={locale} dir={dir}>
      <Head />
      <Preview>Your receipt for {amount} is ready to download.</Preview>
      <Body style={{ backgroundColor: "#ffffff", fontFamily: "Helvetica, Arial, sans-serif" }}>
        <Container style={{ padding: 32 }}>
          <Heading style={{ fontSize: 22 }}>Thanks for your payment</Heading>
          <Section>
            <Text>We received {amount} for {seats} seats on the Pro plan.</Text>
          </Section>
          <Button href={invoiceUrl} style={{ backgroundColor: "#111827", color: "#ffffff", padding: "10px 18px" }}>
            Download invoice
          </Button>
        </Container>
      </Body>
    </Html>
  );
}

InvoicePaid.PreviewProps = {
  locale: "en",
  dir: "ltr",
  amount: "$49.00",
  seats: 5,
  invoiceUrl: "https://acme.example/invoices/inv_42.pdf",
} satisfies InvoicePaidProps;
