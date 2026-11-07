### Messages de l'application (fluent-templates : locales/fr/main.ftl).

## Accueil

# Titre de la page d'accueil
hello-world = Bonjour le monde !
greeting = Bonjour { $name }, bienvenue sur { -brand-name }.

-brand-name = Cliponit
    .gender = masculine

# Nombre d'e-mails non lus
unread-emails =
    { $count ->
        [one] Vous avez un e-mail non lu.
       *[other] Vous avez { $count } e-mails non lus.
    }

login-input = Valeur prédéfinie
    .placeholder = email@exemple.com
    .aria-label = Champ de connexion

## Divers

multiline =
    Première ligne,
    deuxième ligne.

# Indentation irrégulière conservée
shared-photos = { $userName } { $photoCount ->
    [one] a ajouté une photo
   *[other] a ajouté { $photoCount } photos
  } à { $userGender ->
    [male] son flux
    [female] son flux
   *[other] son flux
  }.
price = Prix : { NUMBER($amount, minimumFractionDigits: 2) } €
literal = Accolade { "{" } littérale
only-attrs =
    .title = Infobulle
